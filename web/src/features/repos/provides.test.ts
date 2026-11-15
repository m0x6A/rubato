import { describe, expect, it } from "vitest";
import type { SystemMap } from "../../protocol.ts";
import { realMap } from "../../../test/fixtures.ts";
import { repoFeatures } from "./provides.ts";

// Two repos and an external: auth owns the token seam the shop uses, and a journey crosses both.
const map: SystemMap = {
  name: "acme",
  version: 1,
  nodes: [
    { id: "auth-api", kind: "context", repo: "acme/auth", description: "Issues and checks tokens" },
    { id: "auth-contract", kind: "contract", repo: "acme/auth", description: "Token claims" },
    { id: "shop-web", kind: "context", repo: "acme/shop", description: "The storefront" },
    { id: "payments", kind: "external" },
  ],
  seams: [
    { id: "shop-auth", from: "shop-web", to: "auth-api", owner: "auth-api" },
    { id: "auth-claims", from: "auth-api", to: "auth-contract", owner: "auth-contract" },
    { id: "shop-payments", from: "shop-web", to: "payments", owner: "shop-web" },
  ],
  scenarios: [
    { id: "buy", name: "Buy something", steps: [{ seam: "shop-auth", from: "shop-web", to: "auth-api", kind: "control" }] },
    { id: "claims", name: "Read claims", steps: [{ seam: "auth-claims", from: "auth-api", to: "auth-contract", kind: "data" }] },
  ],
};

describe("what a repo provides, derived from the map (issue #89)", () => {
  it("lists its parts, the seams it owns and uses, and the journeys it takes part in", () => {
    const auth = repoFeatures(map, "acme/auth");
    expect(auth.parts.map((p) => [p.id, p.description])).toEqual([
      ["auth-api", "Issues and checks tokens"],
      ["auth-contract", "Token claims"],
    ]);
    expect(auth.owns.map((s) => s.id)).toEqual(["shop-auth", "auth-claims"]);
    expect(auth.uses.map((s) => s.id)).toEqual([]);
    expect(auth.scenarios.map((s) => s.id)).toEqual(["buy", "claims"]);

    const shop = repoFeatures(map, "acme/shop");
    expect(shop.parts.map((p) => p.id)).toEqual(["shop-web"]);
    expect(shop.owns.map((s) => s.id)).toEqual(["shop-payments"]);
    expect(shop.uses.map((s) => s.id)).toEqual(["shop-auth"]);
    expect(shop.scenarios.map((s) => s.id)).toEqual(["buy"]);
  });

  it("matches the repo without case, and a repo the map does not name provides nothing", () => {
    expect(repoFeatures(map, "ACME/Auth").parts).toHaveLength(2);
    expect(repoFeatures(map, "acme/other")).toEqual({ parts: [], owns: [], uses: [], scenarios: [] });
  });

  it("on the team's own map, rubato provides every part that has a repo", () => {
    const real = realMap();
    const own = repoFeatures(real, "m0x6A/rubato");
    expect(own.parts.length).toBe(real.nodes.filter((n) => n.repo).length);
    expect(own.parts.length).toBeGreaterThan(0);
    // Externals have no repo, so the seams to them are owned here or used from here, never lost.
    expect(own.owns.length + own.uses.length).toBe((real.seams ?? []).length);
  });
});
