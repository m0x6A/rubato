/**
 * The web's client spans (ADR 0082, plan #300): every API call carries a valid `traceparent`
 * and becomes a span with that trace and span id; spans reach the relay within a second, cut at
 * 60 KiB, untraced; a 404 turns tracing off; a 401 from the relay never asks for a login.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onNeedsLogin, request, resetNeedsLogin } from "./api.ts";
import { BATCH_BYTES, FLUSH_MS, RELAY, type ClientSpan, batches, flush, routeOf, setTracing, tracingOn } from "./trace.ts";

const TRACEPARENT = /^00-([0-9a-f]{32})-([0-9a-f]{16})-01$/;

interface Call {
  url: string;
  init: RequestInit;
}

function mockFetch(relayStatus = 202): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      const status = url === RELAY ? relayStatus : 200;
      return new Response(url === RELAY ? null : "{}", { status });
    }),
  );
  return calls;
}

const header = (c: Call, name: string): string | undefined => (c.init.headers as Record<string, string> | undefined)?.[name];
const relayed = (calls: Call[]) => calls.filter((c) => c.url === RELAY);
const spansIn = (c: Call) => JSON.parse(String(c.init.body)).resourceSpans[0].scopeSpans[0].spans as { traceId: string; spanId: string; name: string; attributes: { key: string; value: Record<string, string> }[] }[];

describe("client spans", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setTracing(true);
  });
  afterEach(() => {
    setTracing(false);
    vi.useRealTimers();
    vi.unstubAllGlobals();
    resetNeedsLogin();
  });

  it("an API call carries a valid traceparent and becomes a span with its ids, route and status", async () => {
    const calls = mockFetch();
    await request("GET", "/api/w/shop/pins?x=1");
    const tp = header(calls[0] as Call, "traceparent");
    const [, traceId, spanId] = TRACEPARENT.exec(tp ?? "") ?? [];
    expect(traceId, tp).toBeDefined();
    await vi.advanceTimersByTimeAsync(FLUSH_MS);
    const posts = relayed(calls);
    expect(posts).toHaveLength(1);
    const [s] = spansIn(posts[0] as Call);
    expect(s?.traceId).toBe(traceId);
    expect(s?.spanId).toBe(spanId);
    expect(s?.name).toBe("GET /api/w/{ws}/pins");
    expect(s?.attributes.find((a) => a.key === "http.response.status_code")?.value.intValue).toBe("200");
  });

  it("flushes within a second of the first span, and the post itself is not traced", async () => {
    const calls = mockFetch();
    await request("GET", "/api/overview");
    await vi.advanceTimersByTimeAsync(FLUSH_MS - 1);
    expect(relayed(calls)).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    const [post] = relayed(calls);
    expect(post).toBeDefined();
    expect(header(post as Call, "traceparent")).toBeUndefined();
    expect((post as Call).init.keepalive).toBe(true);
    // Nothing buffered by the post: no further flush ever comes.
    await vi.advanceTimersByTimeAsync(10 * FLUSH_MS);
    expect(relayed(calls)).toHaveLength(1);
  });

  it("a 404 from the relay turns tracing off for the page", async () => {
    const calls = mockFetch(404);
    await request("GET", "/api/overview");
    await vi.advanceTimersByTimeAsync(FLUSH_MS);
    expect(tracingOn()).toBe(false);
    await request("GET", "/api/overview");
    expect(header(calls.at(-1) as Call, "traceparent")).toBeUndefined();
  });

  it("a 401 from the relay does not ask for a login", async () => {
    mockFetch(401);
    const asked = vi.fn();
    onNeedsLogin(asked);
    await request("GET", "/api/overview");
    await vi.advanceTimersByTimeAsync(FLUSH_MS);
    expect(asked).not.toHaveBeenCalled();
  });

  it("a call that never reached the server is a failed span with status 0", async () => {
    const calls: Call[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init: RequestInit) => {
        calls.push({ url, init });
        if (url !== RELAY) {
          throw new TypeError("network down");
        }
        return new Response(null, { status: 202 });
      }),
    );
    await expect(request("GET", "/api/overview")).rejects.toThrow("network down");
    await flush();
    const [s] = spansIn(relayed(calls)[0] as Call) as unknown as { status: { code: number }; attributes: { key: string; value: Record<string, string> }[] }[];
    expect(s?.status.code).toBe(2);
    expect(s?.attributes.find((a) => a.key === "http.response.status_code")?.value.intValue).toBe("0");
  });

  it("off, nothing is added to a request and nothing is sent", async () => {
    setTracing(false);
    const calls = mockFetch();
    await request("GET", "/api/overview");
    await vi.advanceTimersByTimeAsync(10 * FLUSH_MS);
    expect(header(calls[0] as Call, "traceparent")).toBeUndefined();
    expect(relayed(calls)).toHaveLength(0);
  });
});

describe("route templates and batches", () => {
  it("ids, the workspace and the query never reach a span name", () => {
    expect(routeOf("/api/overview")).toBe("/api/overview");
    expect(routeOf("/api/w/shop/map/draft/ops/12")).toBe("/api/w/{ws}/map/draft/ops/{id}");
    expect(routeOf("/api/pulls/m0x6A/rubato/12")).toBe("/api/pulls/{owner}/{repo}/{number}");
    expect(routeOf("/api/launches/9f1c2e/messages?since=3")).toBe("/api/launches/{id}/messages");
    expect(routeOf("/api/drafts/7/revisions/3")).toBe("/api/drafts/{id}/revisions/{id}");
  });

  it("a long buffer is cut into bodies of at most 60 KiB, every span kept", () => {
    const span = (i: number): ClientSpan => ({ traceId: "a".repeat(32), spanId: i.toString(16).padStart(16, "0"), method: "GET", route: "/api/overview", status: 200, startNanos: "1", endNanos: "2", failed: false });
    const all = Array.from({ length: 1500 }, (_, i) => span(i + 1));
    const bodies = batches(all);
    expect(bodies.length).toBeGreaterThan(1);
    expect(bodies.every((b) => b.length <= BATCH_BYTES)).toBe(true);
    const count = bodies.reduce((n, b) => n + JSON.parse(b).resourceSpans[0].scopeSpans[0].spans.length, 0);
    expect(count).toBe(all.length);
  });
});
