import { describe, expect, it } from "vitest";
import { COMPANION_STALE_MS, freshness } from "./Lanes.tsx";

describe("a companion's freshness bar (issue #81)", () => {
  const now = Date.parse("2026-11-13T12:00:00Z");
  const ago = (ms: number) => new Date(now - ms).toISOString();

  it("is full just now, half at five minutes, empty at ten and after", () => {
    expect([freshness(ago(0), now), freshness(ago(COMPANION_STALE_MS / 2), now), freshness(ago(COMPANION_STALE_MS), now), freshness(ago(3 * COMPANION_STALE_MS), now)]).toEqual([1, 0.5, 0, 0]);
  });

  it("is empty for a companion never seen", () => {
    expect(freshness(null, now)).toBe(0);
    expect(freshness(undefined, now)).toBe(0);
  });
});
