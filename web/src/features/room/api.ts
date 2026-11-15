import type { CallInRequest, CallInResponse, DismissAgentRequest, RequestAnswerBody, RoomMessage, RoomPostRequest, RoomView } from "../../protocol.ts";
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
  /** The owner's own launch joins at once; anyone else's call is a request (PR 11). */
  callIn: (req: CallInRequest) => request<CallInResponse>("POST", "/api/room/agents", req),
  /** Its owner, or whoever called it in. */
  dismiss: (req: DismissAgentRequest) => request<unknown>("DELETE", "/api/room/agents", req),
  /** The owner accepts or declines; the asker withdraws. */
  answer: (requestId: string, body: RequestAnswerBody) => request<unknown>("POST", `/api/room/requests/${encodeURIComponent(requestId)}`, body),
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
  not_this_plan: "That session is not one of this plan's on its branch",
  already_in: "That agent is in the room already",
  not_yours: "Only the owner, the asker or whoever called the agent in may do that",
  answered: "That request was answered already",
  companion_cannot: "The owner's companion cannot take room turns yet; it needs an upgrade",
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
