import type { LogKind, SessionLogEntry } from "../../protocol.ts";

/** What a reply is, in words (ADR 0056). */
export const KIND_LABEL: Record<LogKind, string> = {
  prompt: "Prompt",
  reply: "Reply",
  review: "Design review",
  summary: "Discussion summary",
};

/** A prompt and the reply of the same turn: same session, same `prompt_id`. */
export interface Turn {
  prompt: SessionLogEntry;
  reply: SessionLogEntry | null;
}

const turnKey = (e: SessionLogEntry) => `${e.session_id}\n${e.prompt_id}`;

/** The turn each Rubato message started, by message id. A reply of another session never answers it. */
export function turnsByMessage(entries: SessionLogEntry[]): Map<string, Turn> {
  const replies = new Map<string, SessionLogEntry>();
  for (const e of entries) {
    if (e.role === "assistant") {
      replies.set(turnKey(e), e);
    }
  }
  const turns = new Map<string, Turn>();
  for (const e of entries) {
    if (e.role === "user" && e.message_id) {
      turns.set(e.message_id, { prompt: e, reply: replies.get(turnKey(e)) ?? null });
    }
  }
  return turns;
}

const safe = (s: string) => s.replace(/[^A-Za-z0-9_-]/g, "_");

/** The element id of a message in the list, the target of "the message". */
export const messageDomId = (launchId: string, messageId: string) => `msg-${safe(launchId)}-${safe(messageId)}`;

/** The element id of a log entry, the target of "see the reply" and "see it in the conversation". */
export const entryDomId = (launchId: string, e: SessionLogEntry) => `turn-${safe(launchId)}-${safe(e.session_id)}-${safe(e.prompt_id)}-${e.role}`;
