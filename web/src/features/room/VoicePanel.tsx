import { useCallback, useEffect, useRef, useState } from "react";
import type { RoomMessage, VoiceAvailability } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import type { RoomKey } from "./api.ts";
import { roomFailureText } from "./api.ts";
import { classifyJoinError, consented, pickFormat, rememberConsent, repliesToSpeak, UTTERANCE_MAX_SECS, voiceApi } from "./voice.ts";

/** The browser and LiveKit pieces the panel uses, swappable in tests. */
export interface VoiceDeps {
  loadLiveKit: () => Promise<typeof import("livekit-client")>;
  getMicrophone: () => Promise<MediaStream>;
  recorder: (stream: MediaStream, mimeType: string) => MediaRecorder;
  isTypeSupported: (t: string) => boolean;
  audio: (src: string) => HTMLAudioElement;
  secure: () => boolean;
  newId: () => string;
}

const browserDeps: VoiceDeps = {
  loadLiveKit: () => import("livekit-client"),
  getMicrophone: () => navigator.mediaDevices.getUserMedia({ audio: true }),
  recorder: (stream, mimeType) => new MediaRecorder(stream, mimeType ? { mimeType } : {}),
  isTypeSupported: (t) => typeof MediaRecorder !== "undefined" && MediaRecorder.isTypeSupported(t),
  audio: (src) => new Audio(src),
  secure: () => window.isSecureContext,
  newId: () => crypto.randomUUID(),
};

interface Props {
  room: RoomKey;
  voice: VoiceAvailability;
  /** Handles of the agents in the room now. */
  agents: string[];
  messages: RoomMessage[];
  /** A message posted from speech, shown at once. */
  onPosted: (m: RoomMessage) => void;
  deps?: Partial<VoiceDeps>;
}

type Joined = { disconnect: () => Promise<void> | void; since: string };

/**
 * A plan room's voice channel (ADR 0073). People hear each other through the team's LiveKit.
 * Holding *Talk* or *Ask @agent* records one utterance; Rubato transcribes it into the room as you.
 * An agent's reply to a spoken question is played here, a short cut of it; the whole reply is in
 * the room. Speech goes to the provider named below, and through the agents to Anthropic.
 */
export function VoicePanel({ room, voice, agents, messages, onPosted, deps: given }: Props) {
  const deps = { ...browserDeps, ...given };
  const [joined, setJoined] = useState<Joined | null>(null);
  const [joining, setJoining] = useState(false);
  const [people, setPeople] = useState<string[]>([]);
  const [agreed, setAgreed] = useState(consented);
  const [recording, setRecording] = useState<string | null>(null);
  const [said, setSaid] = useState<{ text: string; failed: boolean } | null>(null);
  const rec = useRef<{ r: MediaRecorder; chunks: Blob[]; to: string | null; timer: ReturnType<typeof setTimeout> } | null>(null);
  const playing = useRef<HTMLAudioElement | null>(null);
  const played = useRef(new Set<string>());
  const queue = useRef<string[]>([]);

  const hosts = voice.speech_hosts ?? [];
  const via = hosts.length > 0 ? hosts.join(", ") : "the configured speech provider";

  const join = useCallback(async () => {
    setJoining(true);
    setSaid(null);
    try {
      if (!deps.secure()) {
        throw new Error("insecure");
      }
      const t = await voiceApi.token(room);
      const lk = await deps.loadLiveKit();
      const r = new lk.Room();
      const refresh = () => setPeople([...r.remoteParticipants.values()].map((p) => p.name || p.identity));
      r.on(lk.RoomEvent.ParticipantConnected, refresh);
      r.on(lk.RoomEvent.ParticipantDisconnected, refresh);
      r.on(lk.RoomEvent.TrackSubscribed, (track) => {
        if (track.kind === "audio") {
          document.body.appendChild(track.attach());
        }
      });
      r.on(lk.RoomEvent.Disconnected, () => setJoined(null));
      await r.connect(t.url, t.token);
      await r.localParticipant.setMicrophoneEnabled(true);
      refresh();
      setJoined({ disconnect: () => r.disconnect(), since: new Date().toISOString() });
    } catch (e) {
      const kind = classifyJoinError(e, deps.secure());
      void voiceApi.joinFailed(room, kind);
      const why =
        kind === "insecure"
          ? "the page is not served over HTTPS, so the browser gives no microphone"
          : kind === "permission"
            ? "the microphone was not allowed"
            : e instanceof ApiError
              ? roomFailureText(e)
              : "the voice server could not be reached";
      setSaid({ text: `Could not join voice: ${why} (${kind}).`, failed: true });
    } finally {
      setJoining(false);
    }
    // deps is rebuilt every render; its functions are stable for one panel.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [room]);

  const leave = useCallback(async () => {
    playing.current?.pause();
    await joined?.disconnect();
    setJoined(null);
  }, [joined]);

  // Play each agent reply to a spoken question once, one at a time.
  const playNext = useCallback(() => {
    if (playing.current || queue.current.length === 0) {
      return;
    }
    const id = queue.current.shift() as string;
    const a = deps.audio(voiceApi.speechUrl(id));
    playing.current = a;
    const next = () => {
      playing.current = null;
      playNext();
    };
    a.onended = next;
    a.onerror = next;
    void a.play()?.catch(() => {
      playing.current = null;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!joined || !voice.speech) {
      return;
    }
    for (const m of repliesToSpeak(messages, joined.since)) {
      if (!played.current.has(m.id)) {
        played.current.add(m.id);
        queue.current.push(m.id);
      }
    }
    playNext();
  }, [messages, joined, voice.speech, playNext]);

  const start = async (to: string | null) => {
    if (rec.current || !agreed) {
      return;
    }
    // Talking stops the agent's clip in this browser.
    playing.current?.pause();
    playing.current = null;
    setSaid(null);
    try {
      const stream = await deps.getMicrophone();
      const r = deps.recorder(stream, pickFormat(deps.isTypeSupported));
      const chunks: Blob[] = [];
      r.ondataavailable = (e) => chunks.push(e.data);
      const timer = setTimeout(() => stop(), UTTERANCE_MAX_SECS * 1000);
      rec.current = { r, chunks, to, timer };
      r.start();
      setRecording(to ?? "room");
    } catch (e) {
      setSaid({ text: `No microphone: ${e instanceof Error ? e.message : String(e)}`, failed: true });
    }
  };

  const stop = () => {
    const c = rec.current;
    if (!c) {
      return;
    }
    rec.current = null;
    clearTimeout(c.timer);
    setRecording(null);
    c.r.onstop = () => {
      c.r.stream?.getTracks().forEach((t) => t.stop());
      const blob = new Blob(c.chunks, { type: c.r.mimeType || "audio/webm" });
      void send(blob, c.to);
    };
    c.r.stop();
  };

  const send = async (blob: Blob, to: string | null) => {
    const id = deps.newId();
    setSaid({ text: "Transcribing…", failed: false });
    try {
      const m = await voiceApi.utterance(room, id, blob, to);
      onPosted(m);
      setSaid({ text: to ? `Asked @${to}.` : "Posted to the room.", failed: false });
    } catch (e) {
      setSaid({ text: `Not posted: ${roomFailureText(e)}`, failed: true });
    }
  };

  const hold = (to: string | null) => ({
    onPointerDown: () => void start(to),
    onPointerUp: stop,
    onPointerLeave: () => recording && stop(),
    onKeyDown: (e: React.KeyboardEvent) => {
      if ((e.key === " " || e.key === "Enter") && !e.repeat) {
        e.preventDefault();
        void start(to);
      }
    },
    onKeyUp: (e: React.KeyboardEvent) => {
      if (e.key === " " || e.key === "Enter") {
        stop();
      }
    },
  });

  return (
    <div className="room-voice" data-testid="voice-panel">
      <h4>Voice</h4>
      {!voice.enabled ? (
        <p className="muted">The voice channel is not configured on this server.</p>
      ) : joined ? (
        <>
          <p className="muted" data-testid="voice-people">
            In voice: you{people.length > 0 ? `, ${people.join(", ")}` : ""}
            {voice.speech && <> · speech via {via}</>}
          </p>
          <button type="button" onClick={() => void leave()}>
            Leave voice
          </button>
        </>
      ) : (
        <button type="button" className="primary" disabled={joining} onClick={() => void join()}>
          {joining ? "Joining…" : "Join voice"}
        </button>
      )}
      {voice.speech && !agreed && (
        <div className="room-voice-consent" role="note" data-testid="voice-consent">
          <p>
            Speech you send here is transcribed by {via} and posted in this room, which the whole team reads. Agents called in get it as text, and through them so
            does Anthropic. Agents' replies to spoken questions are spoken by {via} too. A provider outside your own machines may keep what it is sent under its
            own terms. Rubato stores no audio.
          </p>
          <button
            type="button"
            onClick={() => {
              rememberConsent();
              setAgreed(true);
            }}
          >
            I understand
          </button>
        </div>
      )}
      {voice.speech && agreed && (
        <div className="room-voice-talk">
          <button type="button" aria-pressed={recording === "room"} {...hold(null)}>
            {recording === "room" ? "Recording… release to post" : "Hold to talk to the room"}
          </button>
          {agents.map((h) => (
            <button key={h} type="button" aria-pressed={recording === h} {...hold(h)}>
              {recording === h ? "Recording… release to ask" : `Hold to ask @${h}`}
            </button>
          ))}
        </div>
      )}
      <p className={said?.failed ? "warn" : "muted"} role={said?.failed ? "alert" : "status"}>
        {said?.text ?? ""}
      </p>
    </div>
  );
}
