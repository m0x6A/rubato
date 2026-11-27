import { ApiError, failureText, request } from "../../shared/api.ts";
import { inTenant } from "../../shared/workspace.ts";
import type { LaunchLog } from "../../protocol.ts";
import type { LaunchMessages, SessionMessageRequest, SessionMessageView } from "./types.ts";

/** A tenant route: launches are not one workspace's, so the path is `inTenant()`, never `scoped()`. */
export function messagesPath(launchId: string): string {
  return inTenant(`/api/launches/${encodeURIComponent(launchId)}/messages`);
}

/** The launch's log, also a team route (ADR 0052): everyone who may log in reads it. */
export function logPath(launchId: string): string {
  return inTenant(`/api/launches/${encodeURIComponent(launchId)}/log`);
}

export const chatApi = {
  messages: (launchId: string) => request<LaunchMessages>("GET", messagesPath(launchId)),
  log: (launchId: string) => request<LaunchLog>("GET", logPath(launchId)),
  /** Always a new message: "Send again" posts the text, never an old id (ADR 0051). */
  send: (launchId: string, text: string) => request<SessionMessageView>("POST", messagesPath(launchId), { text } satisfies SessionMessageRequest),
  /** The owner stops their own session (ADR 0062): never queued, refused when the companion is away. */
  stop: (launchId: string) => request<SessionMessageView>("POST", stopPath(launchId)),
};

/** A team route like the messages, owner only. */
export function stopPath(launchId: string): string {
  return inTenant(`/api/launches/${encodeURIComponent(launchId)}/stop`);
}

/**
 * What the server said: a refusal's `{ "reason": "<reason>" }`, the shape `POST /api/plans/approve`
 * uses too, or else an `{ "error": "…" }` such as the `unknown api path` of a server without this
 * route. Null when it said nothing readable.
 */
export function serverReason(e: unknown): string | null {
  if (!(e instanceof ApiError)) {
    return null;
  }
  try {
    const v = JSON.parse(e.message) as { reason?: unknown; error?: unknown };
    const said = typeof v.reason === "string" && v.reason ? v.reason : v.error;
    return typeof said === "string" && said ? said : null;
  } catch {
    return null;
  }
}

/** The refusals of the messages route, in words. The code stays beside them, as logs and alerts name it. */
export const REFUSAL_TEXT: Record<string, string> = {
  not_your_session: "Only the person who launched this session can message it",
  too_long: "The message is longer than a session takes",
  empty: "The message is empty",
  unknown_launch: "Rubato does not know this launch",
  companion_offline: "The companion is not connected, so nothing was stopped",
  companion_cannot_stop: "This companion is too old to stop a session; upgrade it",
};

/** Why a call failed, for the person: the refusal in words with its code, or the server's own text. */
export function failureWords(e: unknown): string {
  const reason = serverReason(e);
  if (reason === null) {
    return failureText(e);
  }
  const words = REFUSAL_TEXT[reason];
  return words ? `${words} (${reason})` : reason;
}
