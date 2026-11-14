import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { concernsUs, currentWorkspace, forgetWorkspace, learnWorkspace, scoped, setWorkspace } from "./workspace.ts";

describe("the workspace this page is on (issue #89, ADR 0049)", () => {
  beforeEach(() => {
    history.replaceState(null, "", "/");
    localStorage.clear();
    forgetWorkspace();
  });
  afterEach(() => history.replaceState(null, "", "/"));

  it("starts unprefixed, keeps default unprefixed, and scopes another workspace once named", () => {
    expect(currentWorkspace()).toBeNull();
    expect(scoped("/api/overview")).toBe("/api/overview");
    learnWorkspace("default");
    expect(scoped("/api/pins")).toBe("/api/pins");
    learnWorkspace("shop");
    expect(scoped("/api/pins")).toBe("/api/w/shop/pins");
    // A single workspace keeps a clean URL.
    expect(location.search).toBe("");
  });

  it("takes ?w= first, then the last choice, and a switch writes both", () => {
    localStorage.setItem("rubato.workspace", "admin");
    expect(currentWorkspace()).toBe("admin");
    history.replaceState(null, "", "/?w=shop");
    expect(currentWorkspace()).toBe("shop");
    setWorkspace("admin");
    expect(location.search).toBe("?w=admin");
    expect(localStorage.getItem("rubato.workspace")).toBe("admin");
    expect(scoped("/api/map/draft/ops/3")).toBe("/api/w/admin/map/draft/ops/3");
  });

  it("forgets a workspace the server no longer has", () => {
    setWorkspace("gone");
    forgetWorkspace();
    expect(currentWorkspace()).toBeNull();
    expect(location.search).toBe("");
    expect(localStorage.getItem("rubato.workspace")).toBeNull();
    expect(scoped("/api/overview")).toBe("/api/overview");
  });

  it("only team paths and non-api paths stay as they are", () => {
    setWorkspace("shop");
    expect(scoped("/ws/web")).toBe("/ws/web");
  });

  it("a change for another workspace is not ours; a change for all is", () => {
    learnWorkspace("shop");
    expect(concernsUs({ type: "changed", at: "t", reason: "layout", workspace: "admin" })).toBe(false);
    expect(concernsUs({ type: "changed", at: "t", reason: "layout", workspace: "shop" })).toBe(true);
    expect(concernsUs({ type: "changed", at: "t", reason: "pin" })).toBe(true);
  });
});
