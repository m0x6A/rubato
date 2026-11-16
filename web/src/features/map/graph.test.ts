import { describe, expect, it } from "vitest";
import { issue, overview, realMap } from "../../../test/fixtures.ts";
import { parseIssueKey } from "../../shared/format.ts";
import { findingText } from "../findings/index.ts";
import { buildGraph, issueCounts, layout, layoutByRepo, sessionCounts } from "./graph.ts";

describe("buildGraph from docs/design/rubato-map.yml", () => {
  it("has every live node and seam of the real map, and no retired one (ADR 0077)", () => {
    const g = buildGraph(overview());
    expect(g.nodes.map((n) => n.id).sort()).toEqual(realMap().nodes.filter((n) => n.retired == null).map((n) => n.id).sort());
    expect(g.nodes.length).toBe(12);
    expect(g.edges.length).toBe(13);
    expect(g.nodes.map((n) => n.id)).not.toContain("tmux-ide");
    expect(g.edges.map((e) => e.id)).not.toContain("companion-tmux-ide");
    expect(g.edges.find((e) => e.id === "companion-server")).toMatchObject({ from: "rubato-companion", to: "rubato-server", owner: "rubato-server" });
  });

  it("paints nodes from open plans, strongest status wins, and ignores done issues", () => {
    const planFor = (n: number, status: "new" | "affected" | "quiet", state: "building" | "done") =>
      issue(n, "rubato-companion", {
        progress: { issue: { repo: "m0x6A/rubato", number: n }, state, last_seen: new Date().toISOString() },
        plan: {
          issue: { repo: "m0x6A/rubato", number: n },
          path: `docs/plans/${n}.md`,
          affected_parts: [{ node: "rubato-companion", status, change: "x", seams: ["companion-server"] }],
          o_rows: [{ seam: "companion-server", signal: "s", panel: "p", owner: "rubato-server" }],
        },
      });
    const g = buildGraph(overview({ issues: [planFor(1, "affected", "building"), planFor(2, "new", "building"), planFor(3, "quiet", "done")] }));
    const node = g.nodes.find((n) => n.id === "rubato-companion")!;
    expect(node.paint).toBe("new");
    expect(node.issues.length).toBe(3);
    const edge = g.edges.find((e) => e.id === "companion-server")!;
    expect(edge.openPlans.map((p) => p.number)).toEqual([1, 2]);
    expect(edge.collision).toBe(false);
  });

  it("marks seam collisions and attaches issue findings to pinned nodes", () => {
    const ov = overview({
      issues: [issue(1, "rubato-server")],
      findings: [
        { kind: "seam_collision", seam: "server-github", issues: [{ repo: "m0x6A/rubato", number: 1 }, { repo: "m0x6A/rubato", number: 2 }] },
        { kind: "unknown_ids", issue: { repo: "m0x6A/rubato", number: 1 }, ids: ["rubato-orchestrator"] },
      ],
    });
    const g = buildGraph(ov);
    expect(g.edges.find((e) => e.id === "server-github")?.collision).toBe(true);
    expect(g.nodes.find((n) => n.id === "rubato-server")?.findings.length).toBe(1);
    expect(findingText(ov.findings[0]!)).toContain("two open plans touch server-github");
  });

  it("lays out every node with ELK", async () => {
    const pos = await layout(buildGraph(overview()));
    expect(pos.size).toBe(12);
    const xs = new Set([...pos.values()].map((p) => Math.round(p.x)));
    expect(xs.size).toBeGreaterThan(1);
  });

  it("retiring a part moves no other part, flat and By repo (ADR 0077)", async () => {
    const retired = overview();
    const m = realMap();
    const unretired = overview({ map: { ...retired.map, map: { ...m, nodes: m.nodes.map(({ retired: _, ...n }) => n), seams: (m.seams ?? []).map(({ retired: _, ...s }) => s) } } });
    const before = await layout(buildGraph(unretired));
    const after = await layout(buildGraph(retired));
    expect(after.has("tmux-ide")).toBe(false);
    for (const [id, p] of after) {
      expect(p, id).toEqual(before.get(id));
    }
    const groupedBefore = await layoutByRepo(buildGraph(unretired));
    const groupedAfter = await layoutByRepo(buildGraph(retired));
    expect(groupedAfter.parts.has("tmux-ide")).toBe(false);
    for (const [id, p] of groupedAfter.parts) {
      expect(p, id).toEqual(groupedBefore.parts.get(id));
    }
    console.log(`checked ${after.size + groupedAfter.parts.size} positions with a retired part laid out but not drawn`);
  });

  it("counts and parses", () => {
    expect(issueCounts([issue(1, "web"), issue(2, "web")])).toEqual({ planned: 2 });
    expect(sessionCounts([])).toEqual({});
    expect(parseIssueKey("m0x6A/rubato#7")).toEqual({ repo: "m0x6A/rubato", number: 7 });
    expect(parseIssueKey("nope")).toBeNull();
  });
});
