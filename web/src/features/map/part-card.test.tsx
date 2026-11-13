import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { IssueSummary, SessionState, SessionStatus } from "../../protocol.ts";
import { buildGraph } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";

// Issue #81, design §6: a part at work breathes; a part at rest shows its issues as a strip.

const ref = (n: number) => ({ repo: "m0x6A/rubato", number: n });
function session(n: number, status: SessionStatus): SessionState {
  return { companion_id: "c1", user: "m0x6A", cwd: `/w/${n}`, issue: ref(n), source: "hook", status, last_seen: new Date().toISOString() };
}
function withSession(n: number, status: SessionStatus, extra: Partial<IssueSummary> = {}): IssueSummary {
  return issue(n, "rubato-server", { sessions: [session(n, status)], ...extra });
}
function card(issues: IssueSummary[]) {
  const node = buildGraph(overview({ issues })).nodes.find((n) => n.id === "rubato-server");
  if (!node) throw new Error("no rubato-server part");
  render(<PartCard node={node} selected={false} />);
  return screen.getByTestId("node-rubato-server");
}

describe("a part at work and at rest (issue #81)", () => {
  it("breathes only while a session on it is working", () => {
    expect(card([withSession(1, "working")])).toHaveClass("work");
  });

  for (const status of ["unknown", "idle", "blocked", "ended"] as const) {
    it(`does not breathe for a session that is ${status}: unknown never looks like working`, () => {
      expect(card([withSession(1, status)])).not.toHaveClass("work");
    });
  }

  it("shows its issues as a strip, one segment per state, as wide as its count", () => {
    const c = card([issue(1, "rubato-server"), issue(2, "rubato-server", { progress: { issue: ref(9), state: "building", last_seen: new Date().toISOString() } }), issue(3, "rubato-server", { progress: { issue: ref(9), state: "building", last_seen: new Date().toISOString() } })]);
    const segments = [...c.querySelectorAll(".activity > span")] as HTMLElement[];
    expect(segments.map((s) => [s.dataset["state"], s.style.flexGrow])).toEqual([
      ["planned", "1"],
      ["building", "2"],
    ]);
    expect(c.querySelector(".activity")).toHaveAttribute("aria-hidden", "true");
  });

  it("has no strip without issues, and a glyph for its kind", () => {
    const c = card([]);
    expect(c.querySelector(".activity")).toBeNull();
    expect(c.querySelector("svg.kind-glyph")).not.toBeNull();
  });
});
