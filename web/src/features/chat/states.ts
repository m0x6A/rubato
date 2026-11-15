import type { SessionMessageView } from "./types.ts";

/**
 * A message's state in words (plan 122 §D1). A queued message waits for the session to end its
 * turn while the companion is live, and for the companion otherwise.
 */
export function stateText(m: SessionMessageView, companionLive: boolean): string {
  switch (m.state) {
    case "queued":
      return companionLive ? "waiting for the turn to end" : "waiting for the companion";
    case "sent":
      return "sending…";
    case "delivered":
      return "delivered";
    case "refused":
      return m.detail ? `refused: ${m.detail}` : "refused";
    case "failed":
      return m.detail ? `failed: ${m.detail}` : "failed";
    case "delivery_unknown":
      return "delivery unknown";
  }
}
