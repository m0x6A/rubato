import { describe, expect, it } from "vitest";
import { findingText, windowText } from "./Findings.tsx";

describe("findingText for map versus territory (#104, ADR 0039)", () => {
  it("says a declared seam carried no calls in the report's window", () => {
    expect(findingText({ kind: "seam_quiet", seam: "companion-server", window_secs: 3600 })).toBe(
      "companion-server is on the map but carried no calls in 1 h, while both its parts show traffic",
    );
  });

  it("says which parts talk without a seam, and how often", () => {
    expect(findingText({ kind: "undeclared_traffic", from: "rubato-companion", to: "github", calls: 4 })).toBe(
      "rubato-companion called github 4 times, but no seam on the map joins them",
    );
    expect(findingText({ kind: "undeclared_traffic", from: "a", to: "b", calls: 1 })).toContain("1 time,");
  });

  it("names a window in the largest whole unit", () => {
    expect([60, 90, 1800, 3600, 5400, 86_400].map(windowText)).toEqual(["1 min", "90 s", "30 min", "1 h", "90 min", "1 d"]);
  });
});
