import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onNeedsLogin, resetNeedsLogin } from "../../shared/api.ts";
import { emitWebEvent, onWebEvent } from "./events.ts";
import { subscribe } from "./subscribe.ts";

class FakeSocket {
  static instances: FakeSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((m: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  readonly url: string;
  constructor(url: string) {
    this.url = url;
    FakeSocket.instances.push(this);
  }
  close() {}
}

beforeEach(() => {
  FakeSocket.instances = [];
  resetNeedsLogin();
  vi.useFakeTimers();
  vi.stubGlobal("WebSocket", FakeSocket);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("subscribe (S24)", () => {
  it("a closed socket probes /api/me and routes a 401 to login", async () => {
    const fetchMock = vi.fn(async (_url: string) => new Response("no team record", { status: 401 }));
    vi.stubGlobal("fetch", fetchMock);
    const raised = vi.fn();
    const off = onNeedsLogin(raised);
    const onState = vi.fn();

    const stop = subscribe(() => {}, onState);
    expect(FakeSocket.instances).toHaveLength(1);
    FakeSocket.instances[0]?.onclose?.();
    await vi.advanceTimersByTimeAsync(0);

    expect(onState).toHaveBeenLastCalledWith(false);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/me");
    expect(raised).toHaveBeenCalledTimes(1);

    // Not "disconnected, retrying": no further socket is opened.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(FakeSocket.instances).toHaveLength(1);
    stop();
    off();
  });

  it("a closed socket with a session that still holds reconnects", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ login: "m0x6A" }), { status: 200 })));
    const raised = vi.fn();
    const off = onNeedsLogin(raised);

    const stop = subscribe(() => {}, () => {});
    FakeSocket.instances[0]?.onclose?.();
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeSocket.instances).toHaveLength(2);
    expect(raised).not.toHaveBeenCalled();
    stop();
    off();
  });

  it("a closed socket reconnects when the probe cannot reach the server", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => Promise.reject(new TypeError("network down"))));
    const stop = subscribe(() => {}, () => {});
    FakeSocket.instances[0]?.onclose?.();
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeSocket.instances).toHaveLength(2);
    stop();
  });
});

describe("every event reaches the views that listen (plan 122)", () => {
  it("a frame from the socket goes to onChanged and to every onWebEvent listener, until it stops listening", () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("{}", { status: 200 })));
    const onChanged = vi.fn();
    const heard = vi.fn();
    const off = onWebEvent(heard);
    const stop = subscribe(onChanged, () => {});
    const ev = { type: "updated", at: "2026-11-14T10:00:00Z", resource: { kind: "launch_messages", launch_id: "l1" } };
    FakeSocket.instances[0]?.onmessage?.({ data: JSON.stringify(ev) });
    expect(onChanged).toHaveBeenCalledWith(ev);
    expect(heard).toHaveBeenCalledWith(ev);

    off();
    FakeSocket.instances[0]?.onmessage?.({ data: JSON.stringify(ev) });
    expect(heard).toHaveBeenCalledTimes(1);
    expect(onChanged).toHaveBeenCalledTimes(2);
    stop();
  });

  it("an undecodable frame reaches nobody, and a listener that throws does not stop the others", () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const first = vi.fn(() => {
      throw new Error("boom");
    });
    const second = vi.fn();
    const offFirst = onWebEvent(first);
    const offSecond = onWebEvent(second);
    const stop = subscribe(() => {}, () => {});
    FakeSocket.instances[0]?.onmessage?.({ data: "not json" });
    expect(first).not.toHaveBeenCalled();
    emitWebEvent({ type: "changed", at: "2026-11-14T10:00:00Z", reason: "hook" });
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(1);
    expect(logged).toHaveBeenCalledTimes(1);
    logged.mockRestore();
    offFirst();
    offSecond();
    stop();
  });
});
