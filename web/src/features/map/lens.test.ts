import { describe, expect, it } from "vitest";
import { realMap } from "../../../test/fixtures.ts";
import type { Plan } from "../../protocol.ts";
import { arrowsFor } from "./flows.ts";
import { planLens } from "./lens.ts";

const seams = realMap().seams ?? [];
const plan: Plan = {
  issue: { repo: "m0x6A/rubato", number: 901 },
  path: "docs/plans/901.md",
  affected_parts: [
    { node: "rubato-server", status: "affected", change: "serves one more field", seams: ["web-server"] },
    { node: "plan-file", status: "new", change: "gains a table" },
    { node: "web", status: "quiet", change: "—" },
  ],
  o_rows: [{ seam: "server-plan-file", signal: "s", panel: "p", owner: "plan-file" }],
  flows: [
    { from: "rubato-server", to: "web", seam: "web-server", kind: "data", carries: "the new field" },
    { from: "web", to: "rubato-server", seam: "web-server", kind: "control", carries: "asks for it" },
    { from: "github", to: "rubato-server", seam: "server-github", kind: "data", carries: "open issues" },
    // Not drawable: the seam does not join these, and this seam is not on the map.
    { from: "web", to: "github", seam: "web-server", kind: "data", carries: "nothing real" },
    { from: "web", to: "rubato-server", seam: "no-such-seam", kind: "data", carries: "nothing real" },
  ],
};

describe("the plan lens (S11)", () => {
  it("parts outside the plan are dimmed", () => {
    const lens = planLens(plan, seams);
    expect([...lens.parts]).toEqual([
      ["rubato-server", "affected"],
      ["plan-file", "new"],
      ["web", "quiet"],
    ]);
    // Lit: what the plan names, plus the far end of a flow it declares.
    expect([...lens.litParts].sort()).toEqual(["github", "plan-file", "rubato-server", "web"]);
    for (const outside of ["rubato-companion", "tmux-ide", "claude-code", "skill-rubato", "plan-check", "system-map", "rubato-protocol"]) {
      expect(lens.litParts.has(outside), outside).toBe(false);
    }
    // Seams: from affected parts, from O-rows and from flows.
    expect([...lens.litSeams].sort()).toEqual(["server-github", "server-plan-file", "web-server"]);
  });

  it("plan flows land on their seams", () => {
    const lens = planLens(plan, seams);
    expect([...lens.flows.keys()].sort()).toEqual(["server-github", "web-server"]);
    expect(lens.flows.get("web-server")?.map((f) => `${f.kind} ${f.from}>${f.to} ${f.carries}`)).toEqual(["data rubato-server>web the new field", "control web>rubato-server asks for it"]);
    expect(lens.unplaced).toBe(2);
    // As arrows: both kinds at once, each in its own direction, side by side and moving.
    const webServer = seams.find((s) => s.id === "web-server")!;
    const arrows = arrowsFor({ ...webServer, flows: lens.flows.get("web-server") }, "plan");
    expect(arrows.map((a) => [a.kind, a.reversed, a.label, a.moving])).toEqual([
      ["data", true, "the new field", true],
      ["control", false, "asks for it", true],
    ]);
    expect(arrows[0]?.offset).not.toBe(arrows[1]?.offset);
    // A seam the plan sends nothing over keeps one faint arrow.
    const quiet = seams.find((s) => s.id === "companion-server")!;
    expect(arrowsFor({ ...quiet, flows: lens.flows.get("companion-server") }, "plan")).toMatchObject([{ faint: true, moving: false }]);
  });

  it("a plan without flows still paints its parts", () => {
    const { flows: _flows, ...old } = plan;
    const lens = planLens(old, seams);
    expect(lens.flows.size).toBe(0);
    expect(lens.unplaced).toBe(0);
    expect(lens.parts.size).toBe(3);
    expect([...lens.litSeams].sort()).toEqual(["server-plan-file", "web-server"]);
  });
});
