import type { RoomMessage, RoomPostRequest, RoomView } from "../../protocol.ts";
import { ApiError, failureText, request } from "../../shared/api.ts";

/** Which room: a plan's, by issue and branch (ADR 0057). */
export interface RoomKey {
  repo: string;
  number: number;
  branch: string;
}

/** Team routes: a room is a plan's, not one workspace's, so the paths are never `scoped()`. */
export const roomApi = {
  get: (k: RoomKey) => request<RoomView>("GET", `/api/room?${new URLSearchParams({ repo: k.repo, number: String(k.number), branch: k.branch }).toString()}`),
  /** The new message, 201. */
  post: (req: RoomPostRequest) => request<RoomMessage | undefined>("POST", "/api/room/messages", req),
};

/** What the server said: a refusal's `{ "reason": … }`, else an `{ "error": … }`; null when nothing readable. */
function serverReason(e: unknown): string | null {
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

/** The refusals of the room routes (`rubato_protocol::room::reason`), in words. */
export const REFUSAL_TEXT: Record<string, string> = {
  empty: "The message is empty",
  too_long: "The message is longer than the room takes",
  no_plan: "This branch has no plan or draft, so it has no room",
};

/** Why a call failed: the refusal in words with its code, or what the server said. */
export function roomFailureText(e: unknown): string {
  const reason = serverReason(e);
  if (reason === null) {
    return failureText(e);
  }
  const words = REFUSAL_TEXT[reason];
  return words ? `${words} (${reason})` : reason;
}
