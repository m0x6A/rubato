/**
 * Spans from skills, with no dependencies (ADR 0061). Builds an OTLP/JSON trace export and POSTs
 * it with Node's own `fetch` to the standard OTLP endpoint, once per process, with a short
 * timeout. Silent on every failure: a skill's stdout, stderr and exit code never change.
 *
 * Settings are the standard `OTEL_*` variables. Claude Code strips `OTEL_*` from the processes it
 * spawns, so the companion also exports each one as `RUBATO_OTEL_*`; the standard name wins.
 * `service.namespace` is `rubato` unless the resource attributes set another (ADR 0080).
 *
 * Library:  await runCommand("rubato", "plan check", () => main(argv));
 *           call("github", () => gh([...]));     // a client span with peer.service=github
 * CLI:      node --experimental-strip-types otel-emit.ts span --skill <s> [--name <span>]
 *             [--sub <subcommand>] [--peer <map id>] -- <command> [args…]
 */
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";

/** Span and attribute names, all from `crates/rubato-protocol/src/otel.rs` (checked by a test). */
export const SPAN = {
  command: "rubato.skill.command",
  call: "rubato.skill.call",
} as const;

export const ATTR = {
  skillName: "rubato.skill.name",
  subcommand: "rubato.skill.subcommand",
  exitCode: "rubato.exit_code",
  serviceName: "service.name",
  serviceNamespace: "service.namespace",
  peerService: "peer.service",
} as const;

/** The namespace every part of Rubato emits unless its environment sets another (ADR 0080);
 * `DEFAULT_SERVICE_NAMESPACE` in `crates/rubato-protocol/src/otel.rs` (checked by a test). */
export const DEFAULT_SERVICE_NAMESPACE = "rubato";

/** How long the one POST may take before it is given up. */
export const SEND_TIMEOUT_MS = 800;
/** Spans kept per process; a long `plan.ts wait` drops the rest. */
export const MAX_SPANS = 512;

type Env = Record<string, string | undefined>;
type AttrValue = string | number | boolean;
export type Attrs = Record<string, AttrValue>;

export interface Config {
  url: string;
  headers: Record<string, string>;
  resource: Record<string, string>;
}

/** The standard variable, else its `RUBATO_` mirror. Empty counts as unset. */
function read(env: Env, name: string): string | undefined {
  const v = env[name];
  if (v !== undefined && v !== "") {
    return v;
  }
  const m = env[`RUBATO_${name}`];
  return m !== undefined && m !== "" ? m : undefined;
}

/** `k=v,k2=v2` with percent-encoded values, as OTel's env spec has it. */
export function parsePairs(s: string | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const part of (s ?? "").split(",")) {
    const i = part.indexOf("=");
    if (i <= 0) {
      continue;
    }
    const k = part.slice(0, i).trim();
    let v = part.slice(i + 1).trim();
    try {
      v = decodeURIComponent(v);
    } catch {
      // keep it as written
    }
    if (k) {
      out[k] = v;
    }
  }
  return out;
}

/** Where and how to send, or null when nothing should be sent. */
export function config(env: Env = process.env): Config | null {
  if ((read(env, "OTEL_SDK_DISABLED") ?? "").toLowerCase() === "true") {
    return null;
  }
  if ((read(env, "OTEL_TRACES_EXPORTER") ?? "otlp").split(",").every((e) => e.trim() !== "otlp")) {
    return null;
  }
  const protocol = read(env, "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL") ?? read(env, "OTEL_EXPORTER_OTLP_PROTOCOL");
  if (protocol === "grpc") {
    return null;
  }
  let url = read(env, "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
  if (!url) {
    const base = read(env, "OTEL_EXPORTER_OTLP_ENDPOINT");
    if (!base) {
      return null;
    }
    url = `${base.replace(/\/+$/, "")}/v1/traces`;
  }
  if (!/^https?:\/\//.test(url)) {
    return null;
  }
  return {
    url,
    headers: {
      ...parsePairs(read(env, "OTEL_EXPORTER_OTLP_HEADERS")),
      ...parsePairs(read(env, "OTEL_EXPORTER_OTLP_TRACES_HEADERS")),
    },
    resource: { [ATTR.serviceNamespace]: DEFAULT_SERVICE_NAMESPACE, ...parsePairs(read(env, "OTEL_RESOURCE_ATTRIBUTES")) },
  };
}

/** A W3C `traceparent`, or null when it is not a valid one. */
export function parseTraceparent(s: string | undefined): { traceId: string; spanId: string } | null {
  const m = /^([0-9a-f]{2})-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/.exec((s ?? "").trim());
  if (!m || m[1] === "ff" || /^0+$/.test(m[2] as string) || /^0+$/.test(m[3] as string)) {
    return null;
  }
  return { traceId: m[2] as string, spanId: m[3] as string };
}

interface Recorded {
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  name: string;
  kind: 1 | 3;
  start: bigint;
  end: bigint;
  attrs: Attrs;
  error: boolean;
}

function hex(bytes: number): string {
  return randomBytes(bytes).toString("hex");
}

function otlpValue(v: AttrValue): Record<string, unknown> {
  if (typeof v === "boolean") {
    return { boolValue: v };
  }
  if (typeof v === "number") {
    return Number.isInteger(v) ? { intValue: String(v) } : { doubleValue: v };
  }
  return { stringValue: v };
}

function otlpAttrs(a: Attrs): { key: string; value: Record<string, unknown> }[] {
  return Object.entries(a).map(([key, v]) => ({ key, value: otlpValue(v) }));
}

function isPromise<T>(v: unknown): v is Promise<T> {
  return typeof (v as { then?: unknown } | null)?.then === "function";
}

/** Spans of one process, sent together by `flush`. */
export class Recorder {
  readonly skill: string;
  readonly cfg: Config | null;
  readonly traceId: string;
  readonly parent: string | undefined;
  private readonly spans: Recorded[] = [];
  private readonly wall0 = BigInt(Date.now()) * 1_000_000n;
  private readonly hr0 = process.hrtime.bigint();
  private active: string | undefined;
  dropped = 0;

  constructor(skill: string, env: Env = process.env) {
    this.skill = skill;
    this.cfg = config(env);
    const tp = parseTraceparent(env["TRACEPARENT"]);
    this.traceId = tp?.traceId ?? hex(16);
    this.parent = tp?.spanId;
  }

  /** Nanoseconds since the epoch. */
  now(): bigint {
    return this.wall0 + (process.hrtime.bigint() - this.hr0);
  }

  get enabled(): boolean {
    return this.cfg !== null;
  }

  get recorded(): number {
    return this.spans.length;
  }

  private push(s: Recorded): void {
    if (this.spans.length >= MAX_SPANS) {
      this.dropped++;
      return;
    }
    this.spans.push(s);
  }

  /**
   * Run `fn` inside a span. Spans opened while it runs are its children. A throw, a rejected
   * promise, or `isError(result)` marks the span as failed; `attrsOf(result)` adds attributes.
   */
  span<T>(
    name: string,
    attrs: Attrs,
    fn: () => T,
    opts: { kind?: "internal" | "client"; peer?: string; isError?: (r: Awaited<T>) => boolean; attrsOf?: (r: Awaited<T>) => Attrs } = {},
  ): T {
    if (!this.enabled) {
      return fn();
    }
    const spanId = hex(8);
    const parentSpanId = this.active ?? this.parent;
    const start = this.now();
    const all: Attrs = { ...attrs };
    if (opts.peer) {
      all[ATTR.peerService] = opts.peer;
    }
    const kind: 1 | 3 = opts.kind === "client" || opts.peer ? 3 : 1;
    const before = this.active;
    this.active = spanId;
    const finish = (error: boolean, result?: Awaited<T>) => {
      this.active = before;
      const extra = result !== undefined && opts.attrsOf ? opts.attrsOf(result) : {};
      const rec: Recorded = { traceId: this.traceId, spanId, name, kind, start, end: this.now(), attrs: { ...all, ...extra }, error };
      if (parentSpanId) {
        rec.parentSpanId = parentSpanId;
      }
      this.push(rec);
    };
    let out: T;
    try {
      out = fn();
    } catch (e) {
      finish(true);
      throw e;
    }
    if (isPromise<Awaited<T>>(out)) {
      return out.then(
        (r) => {
          finish(opts.isError ? opts.isError(r) : false, r);
          return r;
        },
        (e: unknown) => {
          finish(true);
          throw e;
        },
      ) as T;
    }
    finish(opts.isError ? opts.isError(out as Awaited<T>) : false, out as Awaited<T>);
    return out;
  }

  /** The OTLP/JSON export request for what was recorded. */
  body(): Record<string, unknown> {
    const resource: Attrs = { ...(this.cfg?.resource ?? {}) };
    resource[ATTR.serviceName] = this.skill;
    return {
      resourceSpans: [
        {
          resource: { attributes: otlpAttrs(resource) },
          scopeSpans: [
            {
              scope: { name: "@rubato/skill" },
              spans: this.spans.map((s) => ({
                traceId: s.traceId,
                spanId: s.spanId,
                ...(s.parentSpanId ? { parentSpanId: s.parentSpanId } : {}),
                name: s.name,
                kind: s.kind,
                startTimeUnixNano: String(s.start),
                endTimeUnixNano: String(s.end),
                attributes: otlpAttrs(s.attrs),
                status: s.error ? { code: 2 } : { code: 0 },
              })),
            },
          ],
        },
      ],
    };
  }

  /** Send everything recorded, once. Never throws, never prints, waits at most the timeout. */
  async flush(): Promise<void> {
    if (!this.cfg || this.spans.length === 0) {
      return;
    }
    const body = JSON.stringify(this.body());
    this.spans.length = 0;
    try {
      const res = await fetch(this.cfg.url, {
        method: "POST",
        headers: { ...this.cfg.headers, "content-type": "application/json" },
        body,
        signal: AbortSignal.timeout(SEND_TIMEOUT_MS),
      });
      await res.body?.cancel().catch(() => undefined);
    } catch {
      // the collector is down, slow or refusing: the skill carries on as if nothing was sent
    }
  }
}

let current: Recorder | null = null;

/** The recorder of the running command, if any (tests and nested helpers). */
export function active(): Recorder | null {
  return current;
}

/**
 * Run one skill command in a `rubato.skill.command` span with its exit code, then send. Returns
 * what `main` returns; a throw is recorded as a failure and thrown again after sending.
 */
export async function runCommand(skill: string, subcommand: string, main: () => Promise<number> | number, env: Env = process.env): Promise<number> {
  const rec = new Recorder(skill, env);
  const before = current;
  current = rec;
  try {
    return await rec.span(SPAN.command, { [ATTR.skillName]: skill, [ATTR.subcommand]: subcommand }, () => Promise.resolve(main()), {
      isError: (code) => code !== 0,
      attrsOf: (code) => ({ [ATTR.exitCode]: code }),
    });
  } finally {
    current = before;
    await rec.flush();
  }
}

/**
 * A call from the running command to another part, as a client span with `peer.service` set to
 * that part's map id. Without a running command, or with nothing configured, just runs `fn`.
 */
export function call<T>(peer: string, fn: () => T): T {
  if (!current) {
    return fn();
  }
  return current.span(SPAN.call, { [ATTR.skillName]: current.skill }, fn, { kind: "client", peer });
}

/** The skill subcommand for a span: the script and its first positional argument. */
export function subcommandOf(script: string, argv: string[]): string {
  const first = argv.find((a) => !a.startsWith("-"));
  return first ? `${script} ${first}` : script;
}

function opt(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
}

/** `span … -- <command>`: run it with inherited stdio and send one span for it. */
export async function cli(argv: string[], env: Env = process.env): Promise<{ code: number; signal: NodeJS.Signals | null }> {
  const dash = argv.indexOf("--");
  const own = dash >= 0 ? argv.slice(0, dash) : argv;
  const cmd = dash >= 0 ? argv.slice(dash + 1) : [];
  if (own[0] !== "span" || cmd.length === 0) {
    process.stderr.write("usage: otel-emit.ts span --skill <name> [--name <span>] [--sub <subcommand>] [--peer <map id>] -- <command> [args…]\n");
    return { code: 2, signal: null };
  }
  const skill = opt(own, "--skill") ?? "skill";
  const peer = opt(own, "--peer");
  const name = opt(own, "--name") ?? (peer ? SPAN.call : SPAN.command);
  const sub = opt(own, "--sub") ?? (cmd[0] as string);
  const rec = new Recorder(skill, env);
  const result = await rec.span(
    name,
    { [ATTR.skillName]: skill, [ATTR.subcommand]: sub },
    () =>
      new Promise<{ code: number; signal: NodeJS.Signals | null }>((done) => {
        const child = spawn(cmd[0] as string, cmd.slice(1), { stdio: "inherit", env: process.env });
        child.on("error", (e) => {
          process.stderr.write(`otel-emit: ${cmd[0]}: ${e.message}\n`);
          done({ code: 127, signal: null });
        });
        child.on("exit", (code, signal) => done({ code: code ?? 128, signal }));
      }),
    {
      ...(peer ? { peer } : {}),
      isError: (r) => r.code !== 0 || r.signal !== null,
      attrsOf: (r) => ({ [ATTR.exitCode]: r.code }),
    },
  );
  await rec.flush();
  return result;
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  const r = await cli(process.argv.slice(2));
  if (r.signal) {
    process.kill(process.pid, r.signal);
  } else {
    process.exitCode = r.code;
  }
}
