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

  it("a box takes its size, its drag and its handle from what is wanted (issue #144)", () => {
    type B = N & { style?: { width: number; height: number }; draggable?: boolean; dragHandle?: string; selected?: boolean };
    const first: B = { ...node("repo:acme/auth", 0, "box", { width: 300, height: 200 }), style: { width: 300, height: 200 }, draggable: false, selected: true };
    const grown: B = { ...node("repo:acme/auth", 0, "box"), style: { width: 500, height: 260 }, draggable: true, dragHandle: ".repo-box-frame" };
    const merged = mergeNodes<B>([first], [grown]);
    expect(merged[0]?.style).toEqual({ width: 500, height: 260 });
    expect(merged[0]?.draggable).toBe(true);
    expect(merged[0]?.dragHandle).toBe(".repo-box-frame");
    // What React Flow learned stays: the measurement and its own selection.
    expect(merged[0]?.measured).toEqual({ width: 300, height: 200 });
    expect(merged[0]?.selected).toBe(true);
    // A property no longer wanted goes.
    const plain = mergeNodes<B>(merged, [node("repo:acme/auth", 0, "box")]);
    expect(plain[0]?.dragHandle).toBeUndefined();
  });
});
