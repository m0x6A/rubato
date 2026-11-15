import { describe, expect, it } from "vitest";
import { mergeNodes } from "./nodes.ts";

type N = { id: string; position: { x: number; y: number }; data: { label: string }; measured?: { width: number; height: number } };
const node = (id: string, x: number, label: string, measured?: N["measured"]): N => ({ id, position: { x, y: 0 }, data: { label }, ...(measured ? { measured } : {}) });

describe("mergeNodes", () => {
  it("a refresh keeps each part's measurement and takes the new position and data", () => {
    const current = [node("a", 0, "old", { width: 220, height: 96 }), node("b", 10, "old", { width: 220, height: 96 })];
    const merged = mergeNodes(current, [node("a", 5, "new"), node("b", 15, "new")]);
    expect(merged.map((n) => n.measured)).toEqual([
      { width: 220, height: 96 },
      { width: 220, height: 96 },
    ]);
    expect(merged.map((n) => n.position.x)).toEqual([5, 15]);
    expect(merged.map((n) => n.data.label)).toEqual(["new", "new"]);
  });

  it("a new part comes in unmeasured and a removed one goes", () => {
    const merged = mergeNodes([node("a", 0, "old", { width: 1, height: 1 }), node("gone", 0, "old", { width: 1, height: 1 })], [node("new", 1, "n"), node("a", 2, "n")]);
    expect(merged.map((n) => n.id)).toEqual(["new", "a"]);
    expect(merged[0]?.measured).toBeUndefined();
    expect(merged[1]?.measured).toEqual({ width: 1, height: 1 });
  });

  it("a part takes its box from what is wanted, and leaves it when the grouping is turned off", () => {
    type P = N & { parentId?: string };
    const inBox: P = { ...node("a", 1, "n"), parentId: "repo:acme/auth" };
    const merged = mergeNodes<P>([node("a", 0, "old", { width: 1, height: 1 })], [inBox]);
    expect(merged[0]?.parentId).toBe("repo:acme/auth");
    const out = mergeNodes<P>(merged, [node("a", 2, "n")]);
    expect(out[0]?.parentId).toBeUndefined();
    expect(out[0]?.measured).toEqual({ width: 1, height: 1 });
  });
});
