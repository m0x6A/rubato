import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { routeOf } from "./trace.ts";
import { currentTenant, forgetTenant, forgetWorkspace, inTenant, learnTenants, learnWorkspace, scoped, setTenant, setWorkspace, workspacePath } from "./workspace.ts";

describe("the tenant this page acts in (issue #345, ADR 0089)", () => {
  beforeEach(() => {
    history.replaceState(null, "", "/");
    localStorage.clear();
    forgetTenant();
    forgetWorkspace();
  });
  afterEach(() => {
    history.replaceState(null, "", "/");
    forgetTenant();
    forgetWorkspace();
  });

  it("takes the first tenant /api/me lists, and every tenant call goes under it", () => {
    expect(currentTenant()).toBeNull();
    expect(learnTenants(["acme", "bcorp"])).toBe(true);
    expect(currentTenant()).toBe("acme");
    expect(inTenant("/api/queue")).toBe("/api/t/acme/queue");
    expect(inTenant("/api/launches/9f/messages")).toBe("/api/t/acme/launches/9f/messages");
    // No workspace learned yet: `default` is asked for, and the overview corrects it.
    expect(scoped("/api/overview")).toBe("/api/t/acme/w/default/overview");
    learnWorkspace("shop");
    expect(scoped("/api/pins")).toBe("/api/t/acme/w/shop/pins");
    expect(workspacePath("shop")).toBe("/api/t/acme/w/shop");
    // Person routes and non-API paths never take a prefix.
    expect(inTenant("/ws/web")).toBe("/ws/web");
    // A single tenant keeps a clean URL.
    expect(location.search).toBe("");
  });

  it("keeps a tenant the link or the last visit named, when the person is in it", () => {
    history.replaceState(null, "", "/?t=bcorp");
    expect(learnTenants(["acme", "bcorp"])).toBe(true);
    expect(currentTenant()).toBe("bcorp");
    expect(scoped("/api/map")).toBe("/api/t/bcorp/w/default/map");
  });

  it("drops a tenant the person is not in, and says when they are in none", () => {
    history.replaceState(null, "", "/?t=bcorp");
    localStorage.setItem("rubato.tenant", "bcorp");
    expect(learnTenants(["acme"])).toBe(true);
    expect(currentTenant()).toBe("acme");
    expect(location.search).toBe("");
    expect(localStorage.getItem("rubato.tenant")).toBeNull();
    forgetTenant();
    expect(learnTenants([])).toBe(false);
    expect(currentTenant()).toBeNull();
  });

  it("a switch of tenant writes the URL and starts its workspace choice anew", () => {
    learnTenants(["acme", "bcorp"]);
    setWorkspace("shop");
    setTenant("bcorp");
    expect(location.search).toBe("?t=bcorp");
    expect(localStorage.getItem("rubato.tenant")).toBe("bcorp");
    expect(localStorage.getItem("rubato.workspace")).toBeNull();
    expect(scoped("/api/pins")).toBe("/api/t/bcorp/w/default/pins");
  });

  it("a server that lists no tenants at all (from before tenants) changes nothing", () => {
    expect(learnTenants(undefined)).toBe(true);
    expect(currentTenant()).toBeNull();
    expect(scoped("/api/pins")).toBe("/api/pins");
    expect(inTenant("/api/queue")).toBe("/api/queue");
  });

  it("span names carry neither the tenant nor the workspace (ADR 0082)", () => {
    expect(routeOf("/api/t/acme/w/shop/pins?x=1")).toBe("/api/t/{tenant}/w/{ws}/pins");
    expect(routeOf("/api/t/acme/queue")).toBe("/api/t/{tenant}/queue");
    expect(routeOf("/api/t/acme/launches/9f2b/messages")).toBe("/api/t/{tenant}/launches/{id}/messages");
    expect(routeOf("/api/t/acme/w/shop/pulls/m0x6A/rubato/12")).toBe("/api/t/{tenant}/w/{ws}/pulls/{owner}/{repo}/{number}");
    expect(routeOf("/api/t/acme/w/shop/map/draft/ops/12")).toBe("/api/t/{tenant}/w/{ws}/map/draft/ops/{id}");
  });
});
