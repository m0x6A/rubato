import { describe, expect, it } from "vitest";
import { issue, overview, realMap } from "../../../test/fixtures.ts";
import type { Overview, SystemMap } from "../../protocol.ts";
import { type PartExplanation, type SeamExplanation, explain, lit } from "./explain.ts";

/** web → server → store, with flows on web-server only. */
function small(): SystemMap {
  return {
    version: 1,
    name: "small",
    nodes: [
      { id: "web", kind: "context", description: "The browser view.", repo: "o/r", path: "web" },
      { id: "server", kind: "context" },
      { id: "store", kind: "external" },
    ],
    seams: [
      {
        id: "web-server",
        from: "web",
        to: "server",
        owner: "server",
        guard: "generated types diffed in CI",
        flows: [
          { kind: "data", from: "server", to: "web", carries: "Overview" },
          { kind: "data", from: "web", to: "server", carries: "PinRequest" },
          { kind: "control", from: "server", to: "web", carries: "WebEvent Changed" },
        ],
      },
      { id: "server-store", from: "server", to: "store", owner: "server" },
    ],
  };
}
const withMap = (partial: Partial<Overview> = {}) => overview({ map: { map: small(), version: "v", loaded_at: new Date().toISOString() }, ...partial });

describe("explaining a part or a seam (S26)", () => {
  it("flows are grouped as in and out with the other part and the seam", () => {
    const server = explain(withMap(), { kind: "node", id: "server" }) as PartExplanation;
    expect(server.incoming.map((l) => [l.flow.carries, l.other, l.seam])).toEqual([["PinRequest", "web", "web-server"]]);
    expect(server.outgoing.map((l) => [l.flow.carries, l.other, l.seam])).toEqual([
      ["Overview", "web", "web-server"],
      ["WebEvent Changed", "web", "web-server"],
    ]);
    // The seam to the store declares nothing: that gap is named.
    expect(server.seamsWithoutFlows).toEqual(["server-store"]);
    const web = explain(withMap(), { kind: "node", id: "web" }) as PartExplanation;
    expect(web.codeUrl).toBe("https://github.com/o/r/tree/HEAD/web");
    expect(web.incoming).toHaveLength(2);
    expect(web.outgoing).toHaveLength(1);
    expect(explain(withMap(), { kind: "node", id: "nope" })).toBeNull();
  });

  it("dependants and dependencies come from seam direction", () => {
    const server = explain(withMap(), { kind: "node", id: "server" }) as PartExplanation;
    expect(server.dependsOn).toEqual([{ part: "store", seam: "server-store" }]);
    expect(server.dependants).toEqual([{ part: "web", seam: "web-server" }]);
    const store = explain(withMap(), { kind: "node", id: "store" }) as PartExplanation;
    expect(store.dependsOn).toEqual([]);
    expect(store.dependants).toEqual([{ part: "server", seam: "server-store" }]);
  });

  it("neighbours stay lit, the rest dims", () => {
    const seams = small().seams ?? [];
    expect(lit(seams, null)).toBeNull();
    const web = lit(seams, { kind: "node", id: "web" });
    expect([...(web?.nodes ?? [])].sort()).toEqual(["server", "web"]);
    expect([...(web?.seams ?? [])]).toEqual(["web-server"]);
    const seam = lit(seams, { kind: "seam", id: "server-store" });
    expect([...(seam?.nodes ?? [])].sort()).toEqual(["server", "store"]);
    expect([...(seam?.seams ?? [])]).toEqual(["server-store"]);
    // On the real map, selecting the server lights every part it has a seam with.
    const real = lit(realMap().seams ?? [], { kind: "node", id: "rubato-server" });
    expect(real?.nodes.has("web") && real.nodes.has("github") && !real.nodes.has("tmux-ide")).toBe(true);
  });

  it("what touches it now: pinned issues, open plans with their change, sessions and decisions", () => {
    const ref = { repo: "o/r", number: 7 };
    const plan = {
      issue: ref,
      path: "docs/plans/7.md",
      affected_parts: [{ node: "server", status: "affected" as const, change: "adds the layout slice", seams: ["web-server"] }],
      o_rows: [{ seam: "web-server", signal: "rubato.layout.writes", panel: "Layout writes", owner: "server" }],
    };
    const decision = { issue: ref, comment_id: 1, title: "Positions on the server", author: "m0x6A", created_at: "2026-11-10T10:00:00Z", url: "https://example.invalid/1" };
    const ov = withMap({
      issues: [
        issue(7, "server", { issue: ref, title: "Shared layout", plan, decisions: [decision], progress: { issue: ref, state: "designed", last_seen: new Date().toISOString() } }),
        // A done issue's plan no longer touches anything.
        issue(8, "store", { plan: { ...plan, issue: { repo: "o/r", number: 8 } }, progress: { issue: { repo: "o/r", number: 8 }, state: "done", last_seen: new Date().toISOString() } }),
      ],
    });
    const server = explain(ov, { kind: "node", id: "server" }) as PartExplanation;
    expect(server.touching.issues.map((i) => i.issue.number)).toEqual([7]);
    expect(server.touching.plans).toEqual([{ issue: ref, title: "Shared layout", change: "adds the layout slice", status: "affected" }]);
    expect(server.touching.decisions.map((d) => d.title)).toEqual(["Positions on the server"]);
    const seam = explain(ov, { kind: "seam", id: "web-server" }) as SeamExplanation;
    expect(seam.flows).toHaveLength(3);
    expect(seam.oRows).toEqual([{ issue: ref, row: plan.o_rows[0] }]);
    expect(seam.touching.plans.map((p) => p.issue.number)).toEqual([7]);
    expect(seam.touching.issues).toEqual([]);
  });
});
