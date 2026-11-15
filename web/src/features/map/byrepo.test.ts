import { describe, expect, it } from "vitest";
import { overview, realMap } from "../../../test/fixtures.ts";
import type { SystemMap } from "../../protocol.ts";
import { NODE_HEIGHT, NODE_WIDTH, buildGraph, layoutByRepo, repoGroupId } from "./graph.ts";

const twoRepos: SystemMap = {
  name: "acme",
  version: 1,
  nodes: [
    { id: "auth-api", kind: "context", repo: "acme/auth" },
    { id: "auth-contract", kind: "contract", repo: "acme/auth" },
    { id: "shop-web", kind: "context", repo: "acme/shop" },
    { id: "payments", kind: "external" },
  ],
  seams: [
    { id: "shop-auth", from: "shop-web", to: "auth-api", owner: "auth-api" },
    { id: "auth-claims", from: "auth-api", to: "auth-contract", owner: "auth-contract" },
    { id: "shop-payments", from: "shop-web", to: "payments", owner: "shop-web" },
  ],
};

describe("the By repo grouping (issue #89)", () => {
  it("boxes each repo's parts together and leaves parts with no repo outside", async () => {
    const g = buildGraph(overview({ map: { map: twoRepos, version: "v", loaded_at: new Date().toISOString() } }));
    expect(g.nodes.find((n) => n.id === "auth-api")?.repo).toBe("acme/auth");
    const out = await layoutByRepo(g);
    expect(out.groups.map((b) => b.repo).sort()).toEqual(["acme/auth", "acme/shop"]);
    expect(out.parts.get("auth-api")?.parent).toBe(repoGroupId("acme/auth"));
    expect(out.parts.get("auth-contract")?.parent).toBe(repoGroupId("acme/auth"));
    expect(out.parts.get("shop-web")?.parent).toBe(repoGroupId("acme/shop"));
    expect(out.parts.get("payments")?.parent).toBeUndefined();
    // Children sit inside their box, at positions relative to it (React Flow's parentId rule).
    const auth = out.groups.find((b) => b.repo === "acme/auth")!;
    for (const id of ["auth-api", "auth-contract"]) {
      const p = out.parts.get(id)!;
      expect(p.x).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.x + NODE_WIDTH).toBeLessThanOrEqual(auth.width);
      expect(p.y + NODE_HEIGHT).toBeLessThanOrEqual(auth.height);
    }
    // Boxes do not overlap each other.
    const [a, b] = out.groups;
    const apart = a!.x + a!.width <= b!.x || b!.x + b!.width <= a!.x || a!.y + a!.height <= b!.y || b!.y + b!.height <= a!.y;
    expect(apart).toBe(true);
  });

  it("on the team's own map, one box holds every part that has a repo", async () => {
    const out = await layoutByRepo(buildGraph(overview()));
    expect(out.groups.map((b) => b.repo)).toEqual(["m0x6A/rubato"]);
    const inside = [...out.parts.values()].filter((p) => p.parent).length;
    expect(inside).toBe(realMap().nodes.filter((n) => n.repo).length);
    expect(out.parts.size).toBe(realMap().nodes.length);
  });
});
