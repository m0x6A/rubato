import { act, configure, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { issue } from "../../../test/fixtures.ts";
import type { IssueSummary, Me, RoomMessage, RoomView, VoiceAvailability } from "../../protocol.ts";
import { RoomSection } from "./index.ts";
import { type VoiceDeps, VoicePanel } from "./VoicePanel.tsx";
import { classifyJoinError, pickFormat, repliesToSpeak } from "./voice.ts";

// Voice off must never load LiveKit: any import of it fails this file.
vi.mock("livekit-client", () => {
  throw new Error("livekit-client was imported");
});

afterEach(() => vi.restoreAllMocks());
beforeEach(() => localStorage.clear());
configure({ asyncUtilTimeout: 5000 });

const REPO = "m0x6A/rubato";
const BRANCH = "issue/146-voice";
const room = { repo: REPO, number: 146, branch: BRANCH };
const before = "2026-11-15T10:00:00.000Z";
const after = "2099-01-01T00:00:00.000Z";
const voice: VoiceAvailability = { enabled: true, speech: true, speech_hosts: ["speaches:8000"] };

function msg(id: string, extra: Partial<RoomMessage> = {}): RoomMessage {
  return { id, author: "carol", author_kind: "person", text: id, at: after, ...extra };
}

/** A LiveKit stand-in: a Room that connects and reports one other participant. */
function fakeLiveKit(connect: () => Promise<void> = async () => undefined) {
  const connected = vi.fn();
  class Room {
    remoteParticipants = new Map([["dave#1", { identity: "dave#1", name: "dave" }]]);
    localParticipant = { setMicrophoneEnabled: vi.fn(async () => undefined) };
    on() {
      return this;
    }
    async connect(url: string, token: string) {
      connected(url, token);
      await connect();
    }
    disconnect() {}
  }
  const lk = { Room, RoomEvent: { ParticipantConnected: "c", ParticipantDisconnected: "d", TrackSubscribed: "t", Disconnected: "x" } };
  return { lk: lk as unknown as typeof import("livekit-client"), connected };
}

/** A recorder stand-in that yields one chunk on stop. */
function fakeRecorder() {
  const started = vi.fn();
  const make = (_s: MediaStream, mimeType: string) => {
    const r = {
      mimeType,
      stream: { getTracks: () => [] },
      ondataavailable: null as ((e: { data: Blob }) => void) | null,
      onstop: null as (() => void) | null,
      start: () => started(mimeType),
      stop: () => {
        r.ondataavailable?.({ data: new Blob(["opus"], { type: mimeType }) });
        r.onstop?.();
      },
    };
    return r as unknown as MediaRecorder;
  };
  return { make, started };
}

function deps(extra: Partial<VoiceDeps> = {}): Partial<VoiceDeps> {
  return {
    loadLiveKit: async () => fakeLiveKit().lk,
    getMicrophone: async () => ({}) as MediaStream,
    recorder: fakeRecorder().make,
    isTypeSupported: (t) => t === "audio/ogg;codecs=opus",
    audio: () => ({ play: vi.fn(async () => undefined), pause: vi.fn() }) as unknown as HTMLAudioElement,
    secure: () => true,
    newId: () => "u-1",
    ...extra,
  };
}

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

describe("voice helpers", () => {
  it("picks the first recording format the browser takes", () => {
    expect(pickFormat((t) => t === "audio/webm;codecs=opus")).toBe("audio/webm;codecs=opus");
    expect(pickFormat((t) => t === "audio/mp4")).toBe("audio/mp4");
    expect(pickFormat(() => false)).toBe("");
  });

  it("names a failed join's kind", () => {
    expect(classifyJoinError(new Error("x"), false)).toBe("insecure");
    expect(classifyJoinError(Object.assign(new Error("denied"), { name: "NotAllowedError" }), true)).toBe("permission");
    expect(classifyJoinError(new Error("could not establish signal connection"), true)).toBe("signal");
    expect(classifyJoinError(new Error("could not establish pc connection"), true)).toBe("ice");
    expect(classifyJoinError(new Error("??"), true)).toBe("other");
  });

  it("speaks only agent replies to spoken turns, newer than the join", () => {
    const ms = [
      msg("q1", { spoken: true, turn_id: "t1" }),
      msg("a1", { author_kind: "agent", author: "m0x6a-agent", turn_id: "t1" }),
      msg("q2", { turn_id: "t2" }),
      msg("a2", { author_kind: "agent", author: "m0x6a-agent", turn_id: "t2" }),
      msg("a0", { author_kind: "agent", author: "m0x6a-agent", turn_id: "t1", at: before }),
    ];
    expect(repliesToSpeak(ms, "2026-11-15T11:00:00.000Z").map((m) => m.id)).toEqual(["a1"]);
  });
});

describe("the voice panel (ADR 0073)", () => {
  it("is not shown, and LiveKit is never loaded, while voice is off", async () => {
    const draft = { draft_id: "d0000000-0000-4000-8000-000000000001", branch: BRANCH, owner: "alice", rev: 1, at: after, plan_sha: "p", bundle_hash: "h", live: true };
    const iss: IssueSummary = issue(146, "web", { draft });
    const view: RoomView = { repo: REPO, number: 146, branch: BRANCH, messages: [] };
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => json(view));
    const me: Me = { login: "bob" };
    render(<RoomSection issue={iss} me={me} maxChars={8000} />);
    await screen.findByText("No messages yet.");
    expect(screen.queryByTestId("voice-panel")).toBeNull();
  });

  it("joins with the minted token and shows who is in", async () => {
    const lk = fakeLiveKit();
    const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async () => json({ url: "wss://lk.example", token: "jwt", room: "rubato-x", identity: "bob#1", expires_at: after }));
    render(<VoicePanel room={room} voice={voice} agents={[]} messages={[]} onPosted={() => undefined} deps={deps({ loadLiveKit: async () => lk.lk })} />);
    fireEvent.click(screen.getByRole("button", { name: "Join voice" }));
    await screen.findByTestId("voice-people");
    expect(lk.connected).toHaveBeenCalledWith("wss://lk.example", "jwt");
    expect(screen.getByTestId("voice-people")).toHaveTextContent("In voice: you, dave · speech via speaches:8000");
    expect(fetch.mock.calls[0]?.[0]).toBe("/api/room/voice/token");
  });

  it("reports a denied microphone as a failed join", async () => {
    const lk = fakeLiveKit(async () => {
      throw Object.assign(new Error("Permission denied"), { name: "NotAllowedError" });
    });
    const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (input) =>
      String(input).endsWith("/token") ? json({ url: "wss://lk.example", token: "jwt", room: "r", identity: "bob#1", expires_at: after }) : new Response(null, { status: 204 }),
    );
    render(<VoicePanel room={room} voice={voice} agents={[]} messages={[]} onPosted={() => undefined} deps={deps({ loadLiveKit: async () => lk.lk })} />);
    fireEvent.click(screen.getByRole("button", { name: "Join voice" }));
    await screen.findByRole("alert");
    const report = fetch.mock.calls.find((c) => String(c[0]).endsWith("/join-failed"));
    expect(JSON.parse(report?.[1]?.body as string)).toMatchObject({ reason: "permission", number: 146 });
  });

  it("asks for consent before the first utterance, then asks an agent with its handle and an id", async () => {
    const rec = fakeRecorder();
    const posted = vi.fn();
    const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async () => json(msg("u-1", { spoken: true }), 201));
    render(<VoicePanel room={room} voice={voice} agents={["m0x6a-agent"]} messages={[]} onPosted={posted} deps={deps({ recorder: rec.make })} />);
    expect(screen.getByTestId("voice-consent")).toHaveTextContent("transcribed by speaches:8000");
    expect(screen.queryByRole("button", { name: "Hold to ask @m0x6a-agent" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "I understand" }));
    const ask = screen.getByRole("button", { name: "Hold to ask @m0x6a-agent" });
    await act(async () => {
      fireEvent.pointerDown(ask);
    });
    expect(rec.started).toHaveBeenCalledWith("audio/ogg;codecs=opus");
    await act(async () => {
      fireEvent.pointerUp(screen.getByRole("button", { name: "Recording… release to ask" }));
    });
    await waitFor(() => expect(posted).toHaveBeenCalled());
    const url = String(fetch.mock.calls[0]?.[0]);
    expect(url).toContain("/api/room/voice/utterance?");
    expect(url).toContain("id=u-1");
    expect(url).toContain("to=m0x6a-agent");
    expect((fetch.mock.calls[0]?.[1]?.headers as Record<string, string>)["Content-Type"]).toBe("audio/ogg;codecs=opus");
  });

  it("plays a reply to a spoken turn once by its URL, never a typed turn's, and talking stops it", async () => {
    localStorage.setItem("rubato.voice.consent", "1");
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => json({ url: "wss://lk.example", token: "jwt", room: "r", identity: "bob#1", expires_at: after }));
    const made: { src: string; pause: ReturnType<typeof vi.fn> }[] = [];
    const audio = (src: string) => {
      const a = { src, play: vi.fn(async () => undefined), pause: vi.fn() };
      made.push(a);
      return a as unknown as HTMLAudioElement;
    };
    const ms = [
      msg("q1", { spoken: true, turn_id: "t1" }),
      msg("a1", { author_kind: "agent", author: "m0x6a-agent", turn_id: "t1" }),
      msg("q2", { turn_id: "t2" }),
      msg("a2", { author_kind: "agent", author: "m0x6a-agent", turn_id: "t2" }),
    ];
    const { rerender } = render(<VoicePanel room={room} voice={voice} agents={["m0x6a-agent"]} messages={[]} onPosted={() => undefined} deps={deps({ audio })} />);
    fireEvent.click(screen.getByRole("button", { name: "Join voice" }));
    await screen.findByTestId("voice-people");
    rerender(<VoicePanel room={room} voice={voice} agents={["m0x6a-agent"]} messages={ms} onPosted={() => undefined} deps={deps({ audio })} />);
    await waitFor(() => expect(made).toHaveLength(1));
    expect(made[0]?.src).toBe("/api/room/voice/speech/a1");
    rerender(<VoicePanel room={room} voice={voice} agents={["m0x6a-agent"]} messages={[...ms]} onPosted={() => undefined} deps={deps({ audio })} />);
    expect(made).toHaveLength(1);
    await act(async () => {
      fireEvent.pointerDown(screen.getByRole("button", { name: "Hold to talk to the room" }));
    });
    expect(made[0]?.pause).toHaveBeenCalled();
  });
});

describe("a voice channel never outlives what shows it (issue #357)", () => {
  /** A LiveKit stand-in whose Room counts its disconnects. */
  function countingLiveKit() {
    const disconnected = vi.fn();
    const { lk } = fakeLiveKit();
    const Base = (lk as unknown as { Room: new () => { disconnect(): void } }).Room;
    class Room extends Base {
      override disconnect() {
        disconnected();
      }
    }
    return { lk: { ...lk, Room } as unknown as typeof import("livekit-client"), disconnected };
  }
  const token = () => vi.spyOn(globalThis, "fetch").mockImplementation(async () => json({ url: "wss://lk.example", token: "jwt", room: "r", identity: "bob#1", expires_at: after }));

  it("says who holds it while joined, with where speech goes and the way out, and says so when left", async () => {
    token();
    const { lk, disconnected } = countingLiveKit();
    const onHeld = vi.fn();
    localStorage.setItem("rubato.voice.consent", "1");
    render(<VoicePanel room={room} voice={voice} agents={[]} messages={[]} onPosted={() => {}} onHeld={onHeld} deps={deps({ loadLiveKit: async () => lk })} />);
    expect(onHeld).toHaveBeenLastCalledWith(null);
    fireEvent.click(screen.getByRole("button", { name: "Join voice" }));
    await waitFor(() => expect(onHeld).toHaveBeenLastCalledWith(expect.objectContaining({ via: "speaches:8000" })));

    // The way out handed up is the panel's own Leave.
    const held = onHeld.mock.lastCall?.[0] as { leave: () => void };
    await act(async () => held.leave());
    await waitFor(() => expect(onHeld).toHaveBeenLastCalledWith(null));
    expect(disconnected).toHaveBeenCalledTimes(1);
  });

  it("disconnects when the panel goes: closing the issue leaves the channel", async () => {
    token();
    const { lk, disconnected } = countingLiveKit();
    const onHeld = vi.fn();
    const view = render(<VoicePanel room={room} voice={voice} agents={[]} messages={[]} onPosted={() => {}} onHeld={onHeld} deps={deps({ loadLiveKit: async () => lk })} />);
    fireEvent.click(screen.getByRole("button", { name: "Join voice" }));
    await waitFor(() => expect(onHeld).toHaveBeenLastCalledWith(expect.objectContaining({ via: "speaches:8000" })));
    expect(disconnected).not.toHaveBeenCalled();
    view.unmount();
    expect(disconnected).toHaveBeenCalledTimes(1);
    expect(onHeld).toHaveBeenLastCalledWith(null);
  });

  it("must fail: a panel that was never joined has nothing to disconnect", () => {
    const { lk, disconnected } = countingLiveKit();
    const view = render(<VoicePanel room={room} voice={voice} agents={[]} messages={[]} onPosted={() => {}} deps={deps({ loadLiveKit: async () => lk })} />);
    view.unmount();
    // The count above would mean nothing if unmounting always disconnected.
    expect(disconnected).not.toHaveBeenCalled();
  });
});
