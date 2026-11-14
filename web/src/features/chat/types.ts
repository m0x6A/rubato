/**
 * The views of `GET` and `POST /api/launches/{launch_id}/messages` (ADR 0051).
 *
 * Mirrored by hand from `crates/rubato-protocol/src/session_message.rs` (`SessionMessageState`,
 * `SessionMessageView`, `LaunchMessages`, `SessionMessageRequest`): `schema.rs` does not export
 * them yet, so `protocol.ts` has no copy. Once it does, replace this file's body with
 * re-exports from `../../protocol.ts`.
 */

/** Where a message is, as the server holds it. Nothing `delivery_unknown` is pushed again. */
export type SessionMessageState = "queued" | "sent" | "delivered" | "refused" | "failed" | "delivery_unknown";

/** One message as the web shows it. */
export interface SessionMessageView {
  id: string;
  launch_id: string;
  /** `chat` for now. */
  kind: string;
  text: string;
  author: string;
  state: SessionMessageState;
  created_at: string;
  pushed_at?: string | null;
  outcome_at?: string | null;
  /** The refusal reason or the error, when there is one. */
  detail?: string | null;
}

/** The messages of one launch, oldest first, and whether its companion can take them. */
export interface LaunchMessages {
  launch_id: string;
  /** The launch's paired user: only they may send. */
  owner: string;
  /** The companion announced `session_input` at its last Hello. False: "upgrade the companion". */
  companion_can_receive: boolean;
  /** The companion is connected and heard from within two heartbeats. */
  companion_live: boolean;
  messages: SessionMessageView[];
}

/** Web → server: send this text to the launch's session. */
export interface SessionMessageRequest {
  text: string;
}
