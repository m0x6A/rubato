/**
 * Reads the spans out of an OTLP/HTTP protobuf trace export, with no protobuf library: only the
 * fields the e2e run compares (plan 287). Field numbers are from opentelemetry-proto
 * `trace/v1/trace.proto`: ExportTraceServiceRequest.resource_spans = 1; ResourceSpans.resource = 1,
 * scope_spans = 2; Resource.attributes = 1; ScopeSpans.spans = 2; Span.trace_id = 1, span_id = 2,
 * parent_span_id = 4, name = 5, kind = 6.
 */

export type OtlpSpan = { service: string; traceId: string; spanId: string; parentSpanId: string; name: string; kind: number };

export const SPAN_KIND_SERVER = 2;
export const SPAN_KIND_CLIENT = 3;

type Field = { no: number; wire: number; bytes: Buffer; value: number };

/** The fields of one message. Throws on a truncated or unknown encoding, so a wrong guess about the body is loud. */
function fields(buf: Buffer): Field[] {
  const out: Field[] = [];
  let i = 0;
  const varint = (): number => {
    let v = 0;
    let shift = 0;
    for (;;) {
      if (i >= buf.length) throw new Error("truncated varint");
      const b = buf[i++]!;
      v += (b & 0x7f) * 2 ** shift;
      if ((b & 0x80) === 0) return v;
      shift += 7;
    }
  };
  while (i < buf.length) {
    const tag = varint();
    const no = Math.floor(tag / 8);
    const wire = tag % 8;
    if (wire === 0) {
      out.push({ no, wire, bytes: Buffer.alloc(0), value: varint() });
    } else if (wire === 2) {
      const len = varint();
      if (i + len > buf.length) throw new Error("truncated field");
      out.push({ no, wire, bytes: buf.subarray(i, i + len), value: 0 });
      i += len;
    } else if (wire === 1 || wire === 5) {
      const len = wire === 1 ? 8 : 4;
      if (i + len > buf.length) throw new Error("truncated fixed field");
      out.push({ no, wire, bytes: buf.subarray(i, i + len), value: 0 });
      i += len;
    } else {
      throw new Error(`wire type ${wire}`);
    }
  }
  return out;
}

const sub = (f: Field[], no: number): Buffer[] => f.filter((x) => x.no === no && x.wire === 2).map((x) => x.bytes);

/** `service.name` of a Resource: KeyValue.key = 1, value = 2; AnyValue.string_value = 1. */
function serviceName(resource: Buffer): string {
  for (const kv of sub(fields(resource), 1)) {
    const f = fields(kv);
    if (sub(f, 1)[0]?.toString("utf8") !== "service.name") continue;
    const value = sub(f, 2)[0];
    return value ? (sub(fields(value), 1)[0]?.toString("utf8") ?? "") : "";
  }
  return "";
}

/** Every span of one `/v1/traces` body, with the service of its resource. */
export function spansOf(body: Buffer): OtlpSpan[] {
  const out: OtlpSpan[] = [];
  for (const rs of sub(fields(body), 1)) {
    const rsFields = fields(rs);
    const resource = sub(rsFields, 1)[0];
    const service = resource ? serviceName(resource) : "";
    for (const scope of sub(rsFields, 2)) {
      for (const span of sub(fields(scope), 2)) {
        const f = fields(span);
        const hex = (no: number) => sub(f, no)[0]?.toString("hex") ?? "";
        out.push({
          service,
          traceId: hex(1),
          spanId: hex(2),
          parentSpanId: hex(4),
          name: sub(f, 5)[0]?.toString("utf8") ?? "",
          kind: f.find((x) => x.no === 6 && x.wire === 0)?.value ?? 0,
        });
      }
    }
  }
  return out;
}

export type CallPair = { client: OtlpSpan; server: OtlpSpan };

/**
 * The calls a service graph would draw between two services: a server span of `to` whose parent
 * is a client span of `from` in the same trace.
 */
export function callPairs(spans: OtlpSpan[], from: { service: string; name: string }, to: { service: string; name: string }): CallPair[] {
  const clients = new Map<string, OtlpSpan>();
  for (const s of spans) {
    if (s.service === from.service && s.name === from.name && s.kind === SPAN_KIND_CLIENT) clients.set(`${s.traceId}/${s.spanId}`, s);
  }
  const pairs: CallPair[] = [];
  for (const s of spans) {
    if (s.service !== to.service || s.name !== to.name || s.kind !== SPAN_KIND_SERVER || !s.parentSpanId) continue;
    const client = clients.get(`${s.traceId}/${s.parentSpanId}`);
    if (client) pairs.push({ client, server: s });
  }
  return pairs;
}

/**
 * The spans of an OTLP/JSON `ExportTraceServiceRequest`, as the server's relay forwards the
 * web's own spans (ADR 0082): ids are hex strings, the kind a number.
 */
export function spansOfJson(body: unknown): OtlpSpan[] {
  type Attr = { key?: string; value?: { stringValue?: string } };
  type Json = { resourceSpans?: { resource?: { attributes?: Attr[] }; scopeSpans?: { spans?: Record<string, unknown>[] }[] }[] };
  const out: OtlpSpan[] = [];
  for (const rs of (body as Json).resourceSpans ?? []) {
    const service = rs.resource?.attributes?.find((a) => a.key === "service.name")?.value?.stringValue ?? "";
    for (const scope of rs.scopeSpans ?? []) {
      for (const s of scope.spans ?? []) {
        out.push({
          service,
          traceId: String(s["traceId"] ?? ""),
          spanId: String(s["spanId"] ?? ""),
          parentSpanId: String(s["parentSpanId"] ?? ""),
          name: String(s["name"] ?? ""),
          kind: Number(s["kind"] ?? 0),
        });
      }
    }
  }
  return out;
}
