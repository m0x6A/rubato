import { describe, expect, it } from "vitest";
import { overview, realMap } from "../../../test/fixtures.ts";
import { type GraphEdge, buildGraph } from "./graph.ts";
import { badgeState, bundleKey, bundlesOf, isOpen } from "./bundles.ts";

const seam = (id: string, from: string, to: string, extra: Partial<GraphEdge> = {}): GraphEdge => ({ id, from, to, owner: to, carries: undefined, flows: [], openPlans: [], collision: false, issues: [], findings: [], ...extra });
const nodes = [
  { id: "a", repo: "acme/one" },
  { id: "b", repo: "acme/one" },
  { id: "c", repo: "acme/two" },
  { id: "x", repo: undefined },
];

describe("bundles of seams inside one repo (issue #144)", () => {
  it("bundles per unordered pair inside one repo, and never a crossing seam or one to an external", () => {
    const out = bundlesOf(nodes, [seam("ab", "a", "b"), seam("ba", "b", "a"), seam("ac", "a", "c"), seam("ax", "a", "x")]);
    expect([...out.keys()]).toEqual([bundleKey("a", "b")]);
    expect(out.get("a|b")?.seams.map((s) => s.id)).toEqual(["ab", "ba"]);
    expect(out.get("a|b")).toMatchObject({ from: "a", to: "b" });
  });

  it("on the team's map, the seams inside m0x6A/rubato are bundled and the four to externals are not", () => {
    const g = buildGraph(overview());
    const out = bundlesOf(g.nodes, g.edges);
    const bundled = [...out.values()].flatMap((b) => b.seams.map((s) => s.id)).sort();
    const repoOf = new Map(realMap().nodes.map((n) => [n.id, n.repo]));
    const inside = (realMap().seams ?? []).filter((s) => repoOf.get(s.from) && repoOf.get(s.from) === repoOf.get(s.to)).map((s) => s.id).sort();
    expect(bundled).toEqual(inside);
    expect(bundled).not.toContain("server-github");
    console.log(`checked ${g.edges.length} seams: ${bundled.length} bundled`);
  });

  it("opens when one of its seams is lit, when hovered, when pinned; otherwise stays closed", () => {
    const b = bundlesOf(nodes, [seam("ab", "a", "b")]).get("a|b")!;
    const none = { lit: null, hovered: new Set<string>(), pinned: new Set<string>() };
    expect(isOpen(b, none)).toBe(false);
    expect(isOpen(b, { ...none, lit: new Set(["ab"]) })).toBe(true);
    expect(isOpen(b, { ...none, lit: new Set(["other"]) })).toBe(false);
    expect(isOpen(b, { ...none, hovered: new Set(["a|b"]) })).toBe(true);
    expect(isOpen(b, { ...none, pinned: new Set(["a|b"]) })).toBe(true);
  });

  it("the badge keeps the strongest warning of its seams", () => {
    const of = (...s: GraphEdge[]) => badgeState(bundlesOf(nodes, s).get("a|b")!, (id) => id === "drafted");
    expect(of(seam("ab", "a", "b"))).toMatchObject({ tone: "", warn: false, draft: false, text: "1 seam" });
    expect(of(seam("ab", "a", "b", { openPlans: [{ repo: "acme/one", number: 1 }] }), seam("ba", "b", "a"))).toMatchObject({ tone: "busy", text: "2 seams" });
    expect(of(seam("ab", "a", "b", { openPlans: [{ repo: "acme/one", number: 1 }] }), seam("ba", "b", "a", { quietFor: 3600 }))).toMatchObject({ tone: "quiet", warn: false });
    expect(of(seam("ab", "a", "b", { collision: true }), seam("ba", "b", "a", { quietFor: 3600 }))).toMatchObject({ tone: "collision", warn: true });
    expect(of(seam("ab", "a", "b", { findings: [{ kind: "flow_off_seam" } as never] }))).toMatchObject({ warn: true });
    expect(of(seam("drafted", "a", "b")).draft).toBe(true);
    expect(of(seam("ab", "a", "b"), seam("ba", "b", "a")).title).toContain("ab, ba");
  });
});
