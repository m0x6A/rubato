/**
 * A plan room's voice channel from the browser (ADR 0073): the join token, one push-to-talk
 * utterance at a time, a failed join reported, and the speech of an agent's reply to a spoken
 * question played from its same-origin URL. `livekit-client` is loaded only on *Join voice*.
 */
import type { RoomMessage, VoiceToken } from "../../protocol.ts";
import { ApiError, request } from "../../shared/api.ts";
import { inTenant } from "../../shared/workspace.ts";
import type { RoomKey } from "./api.ts";

/** `rubato_protocol::voice::limits`. */
export const UTTERANCE_MAX_SECS = 60;
export const UTTERANCE_MAX_BYTES = 2 * 1024 * 1024;

/** The kinds of failed join the server counts (`rubato_protocol::voice::join_reason`). */
export type JoinFailure = "signal" | "ice" | "permission" | "insecure" | "other";

const roomQuery = (k: RoomKey) => ({ repo: k.repo, number: String(k.number), branch: k.branch });

export const voiceApi = {
  token: (k: RoomKey) => request<VoiceToken>("POST", inTenant("/api/room/voice/token"), { repo: k.repo, number: k.number, branch: k.branch }),
  /** The posted message, 201, or the one this id posted before, 200. */
  async utterance(k: RoomKey, id: string, audio: Blob, to: string | null): Promise<RoomMessage> {
    const q = new URLSearchParams({ ...roomQuery(k), id });
    if (to) {
      q.set("to", to);
    }
    const res = await fetch(`${inTenant("/api/room/voice/utterance")}?${q.toString()}`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": audio.type || "audio/webm" },
      body: audio,
    });
    if (!res.ok) {
      throw new ApiError(res.status, await res.text());
    }
    return (await res.json()) as RoomMessage;
  },
  /** Fire and forget: Rubato never sees LiveKit, so this is its only signal of a failed join. */
  joinFailed: (k: RoomKey, reason: JoinFailure) =>
    request<unknown>("POST", inTenant("/api/room/voice/join-failed"), { repo: k.repo, number: k.number, branch: k.branch, reason }).catch(() => undefined),
  speechUrl: (messageId: string) => inTenant(`/api/room/voice/speech/${encodeURIComponent(messageId)}`),
};

/** The first recording format this browser takes: opus in webm, then ogg, then mp4 (Safari). */
export function pickFormat(isTypeSupported: (t: string) => boolean): string {
  for (const t of ["audio/webm;codecs=opus", "audio/ogg;codecs=opus", "audio/mp4"]) {
    if (isTypeSupported(t)) {
      return t;
    }
  }
  return "";
}

/** What kind of failed join an error was. */
export function classifyJoinError(e: unknown, secure: boolean): JoinFailure {
  if (!secure) {
    return "insecure";
  }
  const name = e instanceof Error ? e.name : "";
  const text = e instanceof Error ? e.message : String(e);
  if (name === "NotAllowedError" || name === "SecurityError" || /permission/i.test(text)) {
    return "permission";
  }
  if (/websocket|signal|401|403|token/i.test(text)) {
    return "signal";
  }
  if (/ice|peer|transport|pc connection|udp/i.test(text)) {
    return "ice";
  }
  return "other";
}

/**
 * The agent replies to speak: an agent's message answering a turn that a spoken person's message
 * made, newer than `since`, oldest first.
 */
export function repliesToSpeak(messages: RoomMessage[], since: string): RoomMessage[] {
  const spokenTurns = new Set(messages.filter((m) => m.author_kind === "person" && m.spoken && m.turn_id).map((m) => m.turn_id as string));
  return messages.filter((m) => m.author_kind === "agent" && !!m.turn_id && spokenTurns.has(m.turn_id) && m.at > since);
}

const CONSENT_KEY = "rubato.voice.consent";

/** Whether this browser confirmed the consent text; storage that throws counts as not yet. */
export function consented(): boolean {
  try {
    return localStorage.getItem(CONSENT_KEY) === "1";
  } catch {
    return false;
  }
}

export function rememberConsent(): void {
  try {
    localStorage.setItem(CONSENT_KEY, "1");
  } catch {
    // A private window: the banner shows again next time.
  }
}
