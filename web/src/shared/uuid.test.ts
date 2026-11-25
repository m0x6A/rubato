import { describe, expect, it } from "vitest";
import { type UuidSource, uuid } from "./uuid.ts";

const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe("uuid (issue #304)", () => {
  it("uses crypto.randomUUID when the page has it", () => {
    expect(uuid({ getRandomValues: (a) => a, randomUUID: () => "from-random-uuid" })).toBe("from-random-uuid");
  });

  it("must fail without randomUUID, as on plain http: still a valid v4 id from getRandomValues", () => {
    const insecure: UuidSource = { getRandomValues: (a) => crypto.getRandomValues(a) };
    const ids = Array.from({ length: 200 }, () => uuid(insecure));
    for (const id of ids) expect(id).toMatch(V4);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("sets the version and variant bits whatever the random bytes are", () => {
    expect(uuid({ getRandomValues: (a) => a.fill(0xff) })).toBe("ffffffff-ffff-4fff-bfff-ffffffffffff");
    expect(uuid({ getRandomValues: (a) => a.fill(0) })).toBe("00000000-0000-4000-8000-000000000000");
  });
});
