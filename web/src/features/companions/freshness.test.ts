import { describe, expect, it } from "vitest";
import { DEFAULT_COMPANION_SILENT_SECS, freshness } from "./Lanes.tsx";

describe("a companion's freshness bar (issue #81)", () => {
  const now = Date.parse("2026-11-13T12:00:00Z");
  const ago = (ms: number) => new Date(now - ms).toISOString();
  const staleMs = DEFAULT_COMPANION_SILENT_SECS * 1000;

  it("is full just now, half at five minutes, empty at ten and after", () => {
    expect([freshness(ago(0), now), freshness(ago(staleMs / 2), now), freshness(ago(staleMs), now), freshness(ago(3 * staleMs), now)]).toEqual([1, 0.5, 0, 0]);
  });

  it("drains over the threshold it is given (#105)", () => {
    expect([freshness(ago(0), now, 300_000), freshness(ago(150_000), now, 300_000), freshness(ago(300_000), now, 300_000)]).toEqual([1, 0.5, 0]);
  });

  it("is empty for a companion never seen", () => {
    expect(freshness(null, now)).toBe(0);
    expect(freshness(undefined, now)).toBe(0);
  });
});
