import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { parse as parseYaml } from "yaml";
import { describe, expect, it } from "vitest";
import { realMap, root } from "../../../test/fixtures.ts";
import type { SystemMap } from "../../protocol.ts";
import { arrowsFor } from "./flows.ts";
import { caption, playerReducer, resolveScenario } from "./scenario.ts";

const fixture = parseYaml(readFileSync(resolve(root, "schema/fixtures/maps/scenarios.yml"), "utf8")) as SystemMap;
const seams = (m: SystemMap) => (m.seams ?? []).map((s) => ({ id: s.id, from: s.from, to: s.to, flows: s.flows ?? [] }));

describe("a scenario resolved to the arrows the map draws (ADR 0024)", () => {
  it("each step is the arrow of its one flow in the scenario lens", () => {
    const sc = fixture.scenarios?.find((s) => s.id === "pin-an-issue");
    const r = resolveScenario(sc!, seams(fixture));
    expect(r.unplaced).toBe(0);
    expect(r.steps.map((s) => [s.arrow, s.carries])).toEqual([
      ["web-server#scenario-1", "PinRequest"],
      ["server-store#scenario-0", "rows"],
      // web-server has two server → web control flows; the step's `carries` picks the first.
      ["web-server#scenario-2", "WebEvent Changed"],
      ["web-server#scenario-0", "Overview"],
    ]);
    // Every resolved arrow id is one the scenario lens draws.
    const drawn = new Set(seams(fixture).flatMap((s) => arrowsFor(s, "scenario").map((a) => a.id)));
    expect(r.steps.every((s) => drawn.has(s.arrow))).toBe(true);
  });

  it("a step whose flow is gone is counted as unplaced, not drawn", () => {
    const sc = fixture.scenarios?.find((s) => s.id === "pin-an-issue")!;
    const without = seams(fixture).map((s) => (s.id === "server-store" ? { ...s, flows: [] } : s));
    const r = resolveScenario(sc, without);
    expect(r.unplaced).toBe(1);
    expect(r.steps.map((s) => s.index)).toEqual([0, 2, 3]);
  });

  it("every step of the team's own scenarios resolves", () => {
    const map = realMap();
    expect((map.scenarios ?? []).map((s) => s.id)).toEqual(["start-a-session", "plan-comes-back", "propose-a-map-change"]);
    for (const sc of map.scenarios ?? []) {
      expect(resolveScenario(sc, seams(map)).unplaced, sc.id).toBe(0);
    }
  });

  it("the caption names the step, both ends, the kind and what moves", () => {
    const sc = fixture.scenarios?.find((s) => s.id === "pin-an-issue")!;
    const r = resolveScenario(sc, seams(fixture));
    expect(caption(r, 2)).toBe("Step 3 of 4: server → web, control: WebEvent Changed — every browser is told something changed");
    expect(caption(r, 1)).toBe("Step 2 of 4: server → store, data: rows");
  });
});

describe("the player", () => {
  const start = { step: 0, playing: false };
  it("steps forward and back within the scenario", () => {
    let s = playerReducer(start, { type: "next", total: 3 });
    expect(s.step).toBe(1);
    s = playerReducer(playerReducer(s, { type: "next", total: 3 }), { type: "next", total: 3 });
    expect(s.step).toBe(2);
    s = playerReducer(s, { type: "prev", total: 3 });
    expect(s.step).toBe(1);
    expect(playerReducer(start, { type: "prev", total: 3 }).step).toBe(0);
  });

  it("plays to the end and stops there; play at the end starts over", () => {
    let s = playerReducer(start, { type: "play", total: 2 });
    expect(s.playing).toBe(true);
    s = playerReducer(s, { type: "tick", total: 2 });
    expect(s).toEqual({ step: 1, playing: true });
    s = playerReducer(s, { type: "tick", total: 2 });
    expect(s).toEqual({ step: 1, playing: false });
    expect(playerReducer(s, { type: "play", total: 2 })).toEqual({ step: 0, playing: true });
    expect(playerReducer({ step: 1, playing: true }, { type: "pause", total: 2 }).playing).toBe(false);
    expect(playerReducer({ step: 3, playing: true }, { type: "reset", total: 2 })).toEqual({ step: 0, playing: false });
    // A jump from the step list stops playing and stays inside the scenario.
    expect(playerReducer({ step: 0, playing: true }, { type: "goto", total: 4, step: 2 })).toEqual({ step: 2, playing: false });
    expect(playerReducer(start, { type: "goto", total: 4, step: 9 }).step).toBe(3);
  });
});
