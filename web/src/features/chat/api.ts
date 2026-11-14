import { ApiError, failureText, request } from "../../shared/api.ts";
import type { LaunchMessages, SessionMessageRequest, SessionMessageView } from "./types.ts";

/** A team route: launches are not one workspace's, so the path is never `scoped()`. */
export function messagesPath(launchId: string): string {
  return `/api/launches/${encodeURIComponent(launchId)}/messages`;
}

export const chatApi = {
  messages: (launchId: string) => request<LaunchMessages>("GET", messagesPath(launchId)),
  /** Always a new message: "Send again" posts the text, never an old id (ADR 0051). */
  send: (launchId: string, text: string) => request<SessionMessageView>("POST", messagesPath(launchId), { text } satisfies SessionMessageRequest),
};

/** What the server said, from its JSON answer `{ "error": "<reason>" }`; null when it said nothing readable. */
export function serverReason(e: unknown): string | null {
  if (!(e instanceof ApiError)) {
    return null;
  }
  try {
    const v = JSON.parse(e.message) as { error?: unknown };
    return typeof v.error === "string" && v.error ? v.error : null;
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
