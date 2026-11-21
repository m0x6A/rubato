/** What `uuid` needs from `crypto`; `randomUUID` is missing outside a secure context. */
export interface UuidSource {
  getRandomValues(bytes: Uint8Array<ArrayBuffer>): Uint8Array<ArrayBuffer>;
  randomUUID?: () => string;
}

/**
 * A random v4 UUID, also outside a secure context (issue #304). Browsers expose
 * `crypto.randomUUID` only on https or localhost; a team serving Rubato over plain http on its
 * LAN has `crypto.getRandomValues` and nothing else.
 */
export function uuid(c: UuidSource = crypto): string {
  if (typeof c.randomUUID === "function") {
    return c.randomUUID();
  }
  const b = Array.from(c.getRandomValues(new Uint8Array(16)));
  b[6] = ((b[6] ?? 0) & 0x0f) | 0x40; // version 4
  b[8] = ((b[8] ?? 0) & 0x3f) | 0x80; // RFC 4122 variant
  const h = b.map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
