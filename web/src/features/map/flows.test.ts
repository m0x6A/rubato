import { describe, expect, it } from "vitest";
import { realMap } from "../../../test/fixtures.ts";
import type { Flow } from "../../protocol.ts";
import { ARROW_GAP, arrowsFor, flowCounts } from "./flows.ts";

const flow = (kind: Flow["kind"], from: string, to: string, carries: string): Flow => ({ kind, from, to, carries });
const seam = {
  id: "web-server",
  from: "web",
  to: "server",
  flows: [flow("data", "server", "web", "Overview"), flow("data", "web", "server", "PinRequest"), flow("control", "server", "web", "WebEvent Changed")],
};

describe("arrows from a seam and a lens (S9)", () => {
  it("one arrow per flow, data solid, control dashed", () => {
    const data = arrowsFor(seam, "data");
    expect(data.map((a) => [a.label, a.reversed, a.dashed])).toEqual([
      // server → web runs against the seam (web → server): the head sits at the seam's from end.
      ["Overview", true, false],
      ["PinRequest", false, false],
    ]);
    const control = arrowsFor(seam, "control");
    expect(control.map((a) => [a.label, a.reversed, a.dashed])).toEqual([["WebEvent Changed", true, true]]);
    expect(new Set([...data, ...control].map((a) => a.id)).size).toBe(3);
    expect([...data, ...control].every((a) => a.seam === "web-server" && !a.faint)).toBe(true);
  });

  it("two flows on one seam get different offsets", () => {
    const offsets = arrowsFor(seam, "data").map((a) => a.offset);
    expect(offsets).toEqual([-ARROW_GAP / 2, ARROW_GAP / 2]);
    // One flow sits on the seam's own line; three spread evenly around it.
    expect(arrowsFor(seam, "control").map((a) => a.offset)).toEqual([0]);
    const three = { ...seam, flows: [seam.flows[0]!, seam.flows[1]!, flow("data", "web", "server", "MapLayout")] };
    expect(arrowsFor(three, "data").map((a) => a.offset)).toEqual([-ARROW_GAP, 0, ARROW_GAP]);
  });

  it("the scenario lens draws every flow standing still; only the plan lens moves (proposed)", () => {
    const sc = arrowsFor(seam, "scenario");
    expect(sc.map((a) => [a.id, a.kind, a.dashed, a.moving])).toEqual([
      ["web-server#scenario-0", "data", false, false],
      ["web-server#scenario-1", "data", false, false],
      ["web-server#scenario-2", "control", true, false],
    ]);
    // The plan lens is unchanged: every arrow dashed and moving.
    expect(arrowsFor(seam, "plan").every((a) => a.dashed && a.moving)).toBe(true);
  });

  it("structure lens gives one arrow per seam", () => {
    expect(arrowsFor(seam, "structure")).toEqual([{ id: "web-server", seam: "web-server", kind: "structure", reversed: false, label: undefined, offset: 0, dashed: false, faint: false, moving: false }]);
    expect(arrowsFor({ id: "a-b", from: "a", to: "b" }, "structure")).toHaveLength(1);
  });

  it("a seam with no flow of the lens's kind keeps one faint plain arrow", () => {
    const none = arrowsFor({ id: "a-b", from: "a", to: "b" }, "data");
    expect(none).toHaveLength(1);
    expect(none[0]).toMatchObject({ id: "a-b", kind: "structure", faint: true, label: undefined });
    const onlyData = { id: "a-b", from: "a", to: "b", flows: [flow("data", "a", "b", "x")] };
    expect(arrowsFor(onlyData, "control")[0]?.faint).toBe(true);
  });

  it("counts the flows of the team's own map", () => {
    const seams = realMap().seams ?? [];
    const counts = flowCounts(seams);
    expect(counts.data + counts.control).toBe(seams.reduce((n, s) => n + (s.flows?.length ?? 0), 0));
    expect(counts.data).toBeGreaterThan(0);
    expect(counts.control).toBeGreaterThan(0);
  });
});
