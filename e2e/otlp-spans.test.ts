import assert from "node:assert/strict";
import { test } from "node:test";
import { callPairs, spansOf, SPAN_KIND_CLIENT, SPAN_KIND_SERVER } from "./otlp-spans.ts";

// A hand-built export, so the reader is tested against the wire format and not against itself.
const varint = (n: number): Buffer => {
  const out: number[] = [];
  do {
    const b = n % 128;
    n = Math.floor(n / 128);
    out.push(n > 0 ? b | 0x80 : b);
  } while (n > 0);
  return Buffer.from(out);
};
const bytes = (no: number, b: Buffer): Buffer => Buffer.concat([varint(no * 8 + 2), varint(b.length), b]);
const str = (no: number, s: string): Buffer => bytes(no, Buffer.from(s, "utf8"));
const int = (no: number, v: number): Buffer => Buffer.concat([varint(no * 8), varint(v)]);
const fixed64 = (no: number): Buffer => Buffer.concat([varint(no * 8 + 1), Buffer.alloc(8, 1)]);

const TRACE = "4bf92f3577b34da6a3ce929d0e0e4736";
const span = (o: { trace?: string; id: string; parent?: string; name: string; kind: number }): Buffer =>
  bytes(
    2,
    Buffer.concat([
      bytes(1, Buffer.from(o.trace ?? TRACE, "hex")),
      bytes(2, Buffer.from(o.id, "hex")),
      ...(o.parent ? [bytes(4, Buffer.from(o.parent, "hex"))] : []),
      str(5, o.name),
      int(6, o.kind),
      fixed64(7), // start_time_unix_nano: a field the reader skips
    ]),
  );
const exported = (service: string, spans: Buffer[]): Buffer =>
  bytes(
    1,
    Buffer.concat([
      bytes(1, Buffer.concat([bytes(1, Buffer.concat([str(1, "service.namespace"), bytes(2, str(1, "rubato"))])), bytes(1, Buffer.concat([str(1, "service.name"), bytes(2, str(1, service))]))])),
      bytes(2, Buffer.concat([bytes(1, str(1, "scope")), ...spans])),
    ]),
  );

const FROM = { service: "rubato-companion", name: "rubato.envelope.send" };
const TO = { service: "rubato-server", name: "rubato.inbound_event" };

test("a server span under a client span of the other service in the same trace is one call", () => {
  const companion = spansOf(exported("rubato-companion", [span({ id: "00f067aa0ba902b7", name: FROM.name, kind: SPAN_KIND_CLIENT })]));
  const server = spansOf(exported("rubato-server", [span({ id: "1111111111111111", parent: "00f067aa0ba902b7", name: TO.name, kind: SPAN_KIND_SERVER })]));
  assert.deepEqual(companion, [{ service: "rubato-companion", traceId: TRACE, spanId: "00f067aa0ba902b7", parentSpanId: "", name: FROM.name, kind: SPAN_KIND_CLIENT }]);
  const pairs = callPairs([...companion, ...server], FROM, TO);
  assert.equal(pairs.length, 1);
  assert.equal(pairs[0]!.server.spanId, "1111111111111111");
});

test("must fail: a server span with no parent, another trace, or the wrong kind is not a call", () => {
  const client = spansOf(exported("rubato-companion", [span({ id: "00f067aa0ba902b7", name: FROM.name, kind: SPAN_KIND_CLIENT })]));
  const cases: [string, Buffer][] = [
    ["no parent", span({ id: "1111111111111111", name: TO.name, kind: SPAN_KIND_SERVER })],
    ["another trace", span({ trace: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", id: "1111111111111111", parent: "00f067aa0ba902b7", name: TO.name, kind: SPAN_KIND_SERVER })],
    ["internal kind", span({ id: "1111111111111111", parent: "00f067aa0ba902b7", name: TO.name, kind: 1 })],
    ["another parent", span({ id: "1111111111111111", parent: "2222222222222222", name: TO.name, kind: SPAN_KIND_SERVER })],
  ];
  for (const [why, s] of cases) {
    assert.deepEqual(callPairs([...client, ...spansOf(exported("rubato-server", [s]))], FROM, TO), [], why);
  }
  console.log(`checked ${cases.length} shapes that are not a call`);
});

test("a truncated body is an error, not an empty list", () => {
  const body = exported("rubato-server", [span({ id: "1111111111111111", name: TO.name, kind: SPAN_KIND_SERVER })]);
  assert.throws(() => spansOf(body.subarray(0, body.length - 3)));
});
