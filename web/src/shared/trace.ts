/**
 * The web's own client spans (ADR 0082): each API request the web makes is a span, sent with a
 * `traceparent` so the server's span for it sits under it, and posted to the server's relay
 * within a second, so the service graph sees both ends of the call and draws `web → rubato-server`.
 *
 * No dependency: a span is a few fields, and the batch is OTLP/JSON. The relay rewrites what it
 * gets and keeps only ids, kind, times, the status code, the method, the route template and the
 * HTTP status; nothing else is sent. The post is a plain `fetch`, never traced itself, and a 404
 * (the server exports no traces) turns tracing off until the page reloads.
 */

/** The relay's route and the browser's limits; the same numbers as `rubato_protocol::telemetry_relay`. */
export const RELAY = "/api/telemetry/traces";
export const FLUSH_MS = 1000;
export const BATCH_BYTES = 60 * 1024;

export interface ClientSpan {
  traceId: string;
  spanId: string;
  method: string;
  route: string;
  status: number;
  startNanos: string;
  endNanos: string;
  failed: boolean;
}

interface State {
  enabled: boolean;
  buffer: ClientSpan[];
  timer: ReturnType<typeof setTimeout> | null;
}

// Off under the unit tests, whose fetch mocks count every call; a test turns it on itself.
const state: State = { enabled: import.meta.env?.MODE !== "test", buffer: [], timer: null };

/** Turn tracing on or off and drop anything buffered (tests, and a 404 from the relay). */
export function setTracing(enabled: boolean): void {
  state.enabled = enabled;
  state.buffer = [];
  if (state.timer) {
    clearTimeout(state.timer);
    state.timer = null;
  }
}

export function tracingOn(): boolean {
  return state.enabled;
}

function hex(bytes: number): string {
  const b = new Uint8Array(bytes);
  crypto.getRandomValues(b);
  // An all-zero id is invalid; the chance is nil, but make it impossible.
  if (b.every((x) => x === 0)) {
    b[bytes - 1] = 1;
  }
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}

/** Wall-clock nanoseconds as a decimal string: OTLP/JSON's form, past what a number holds. */
function nowNanos(): string {
  const ms = performance.timeOrigin + performance.now();
  return (BigInt(Math.floor(ms * 1000)) * 1000n).toString();
}

/** A literal segment the relay accepts: a lowercase word. Anything else is an id. */
const WORD = /^[a-z_-]+$/;

/**
 * The route template of an API path (ADR 0082): the query dropped, the workspace and every id
 * replaced by a placeholder, so span names stay few and say nothing about whose data it was.
 * `/api/t/acme/w/shop/pins` → `/api/t/{tenant}/w/{ws}/pins`; `/api/pulls/m0x6A/rubato/12` →
 * `/api/pulls/{owner}/{repo}/{number}`; `/api/launches/9f…/messages` → `/api/launches/{id}/messages`.
 */
export function routeOf(url: string): string {
  const path = url.split(/[?#]/)[0] ?? "";
  const segs = path.split("/").slice(1);
  if (segs[0] !== "api") {
    return path;
  }
  // ADR 0089: the tenant and the workspace are never part of a span name.
  let at = 1;
  if (segs[at] === "t" && segs[at + 1] !== undefined) {
    segs[at + 1] = "{tenant}";
    at += 2;
  }
  if (segs[at] === "w" && segs[at + 1] !== undefined) {
    segs[at + 1] = "{ws}";
    at += 2;
  }
  if (segs[at] === "pulls" && segs.length === at + 4) {
    return "/" + [...segs.slice(0, at), "pulls", "{owner}", "{repo}", "{number}"].join("/");
  }
  return "/" + segs.map((s, i) => (i === 0 || WORD.test(s) || s.startsWith("{") ? s : "{id}")).join("/");
}

/** The open span of one request: its header to send, and how to finish it. */
export interface Started {
  traceparent: string;
  end: (status: number) => void;
}

/** Start a span for `method url`, or `null` when tracing is off or the URL is not the API's. */
export function startSpan(method: string, url: string): Started | null {
  if (!state.enabled || !url.startsWith("/api/") || url.startsWith(RELAY)) {
    return null;
  }
  const traceId = hex(16);
  const spanId = hex(8);
  const startNanos = nowNanos();
  const route = routeOf(url);
  return {
    traceparent: `00-${traceId}-${spanId}-01`,
    end: (status: number) => {
      record({ traceId, spanId, method: method.toUpperCase(), route, status, startNanos, endNanos: nowNanos(), failed: status === 0 || status >= 500 });
    },
  };
}

function record(span: ClientSpan): void {
  if (!state.enabled) {
    return;
  }
  state.buffer.push(span);
  if (!state.timer) {
    state.timer = setTimeout(() => void flush(), FLUSH_MS);
  }
}

/** One span as OTLP/JSON, with only what the relay keeps. */
function otlpSpan(s: ClientSpan): unknown {
  return {
    traceId: s.traceId,
    spanId: s.spanId,
    name: `${s.method} ${s.route}`,
    kind: 3,
    startTimeUnixNano: s.startNanos,
    endTimeUnixNano: s.endNanos,
    attributes: [
      { key: "http.request.method", value: { stringValue: s.method } },
      { key: "url.path", value: { stringValue: s.route } },
      { key: "http.response.status_code", value: { intValue: String(s.status) } },
    ],
    status: { code: s.failed ? 2 : 0 },
  };
}

/** The OTLP/JSON body for `spans`. The relay sets the resource again; this one says what it is. */
export function batchBody(spans: ClientSpan[]): string {
  return JSON.stringify({
    resourceSpans: [
      {
        resource: { attributes: [{ key: "service.name", value: { stringValue: "web" } }] },
        scopeSpans: [{ scope: { name: "rubato-web" }, spans: spans.map(otlpSpan) }],
      },
    ],
  });
}

/** Split `spans` into bodies of at most `BATCH_BYTES`, in order. A span alone is never too big. */
export function batches(spans: ClientSpan[]): string[] {
  const out: string[] = [];
  let cur: ClientSpan[] = [];
  for (const s of spans) {
    if (cur.length > 0 && batchBody([...cur, s]).length > BATCH_BYTES) {
      out.push(batchBody(cur));
      cur = [];
    }
    cur.push(s);
  }
  if (cur.length > 0) {
    out.push(batchBody(cur));
  }
  return out;
}

/** Post what is buffered, untraced, with `keepalive` so a flush on `pagehide` still goes out. */
export async function flush(): Promise<void> {
  if (state.timer) {
    clearTimeout(state.timer);
    state.timer = null;
  }
  const spans = state.buffer;
  state.buffer = [];
  if (!state.enabled || spans.length === 0) {
    return;
  }
  for (const body of batches(spans)) {
    try {
      const res = await fetch(RELAY, { method: "POST", keepalive: true, credentials: "same-origin", headers: { "content-type": "application/json" }, body });
      if (res.status === 404) {
        // The server exports no traces: stop for this page.
        setTracing(false);
        return;
      }
    } catch {
      // Dropped: telemetry never retries and never bothers the person.
    }
  }
}

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => void flush());
}
