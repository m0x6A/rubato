import { getBezierPath, Position } from "@xyflow/react";
import { describe, expect, it } from "vitest";
import { type LabelStack, type Rect, bezierAt, placeLabels, stackAt } from "./labels.ts";

const chipOnly = (key: string, from: string, to: string): LabelStack => ({ key, from, to, arrows: [{ offset: 0, label: null }], chip: { width: 60, height: 22 }, chipAbove: 24 });

describe("where a seam's labels sit in the By repo grouping (issue #144)", () => {
  it("the curve is React Flow's: at t = 0.5 it is getBezierPath's label point", () => {
    const s = { x: 100, y: 40 };
    const e = { x: 400, y: 260 };
    const [, lx, ly] = getBezierPath({ sourceX: s.x, sourceY: s.y, sourcePosition: Position.Right, targetX: e.x, targetY: e.y, targetPosition: Position.Left });
    const p = bezierAt(s, e, 0.5);
    expect(p.x).toBeCloseTo(lx, 6);
    expect(p.y).toBeCloseTo(ly, 6);
    expect(bezierAt(s, e, 0)).toEqual(s);
    expect(bezierAt(s, e, 1)).toEqual(e);
  });

  it("a label with room at the middle stays there", () => {
    const cards = new Map<string, Rect>([
      ["a", { x: 0, y: 0, width: 200, height: 100 }],
      ["b", { x: 500, y: 0, width: 200, height: 100 }],
    ]);
    expect(placeLabels([chipOnly("ab", "a", "b")], cards).get("ab")).toBe(0.5);
  });

  it("must fail without it: a label whose middle lies on a card moves along its curve off the card", () => {
    // a → c passes over b, which sits where the middle of the curve is.
    const cards = new Map<string, Rect>([
      ["a", { x: 0, y: 0, width: 200, height: 100 }],
      ["b", { x: 380, y: 0, width: 200, height: 100 }],
      ["c", { x: 900, y: 0, width: 200, height: 100 }],
    ]);
    const t = placeLabels([chipOnly("ac", "a", "c")], cards).get("ac")!;
    expect(t).not.toBe(0.5);
    const [chip] = stackAt(chipOnly("ac", "a", "c"), { x: 200, y: 50 }, { x: 900, y: 50 }, t);
    const b = cards.get("b")!;
    expect(chip!.x + chip!.width <= b.x || chip!.x >= b.x + b.width).toBe(true);
  });

  it("two curves that cross keep their labels apart", () => {
    const cards = new Map<string, Rect>([
      ["a", { x: 0, y: 0, width: 200, height: 100 }],
      ["b", { x: 0, y: 200, width: 200, height: 100 }],
      ["c", { x: 400, y: 0, width: 200, height: 100 }],
      ["d", { x: 400, y: 200, width: 200, height: 100 }],
    ]);
    // a → d and b → c cross in the middle.
    const placed = placeLabels([chipOnly("ad", "a", "d"), chipOnly("bc", "b", "c")], cards);
    const [r1] = stackAt(chipOnly("ad", "a", "d"), { x: 200, y: 50 }, { x: 400, y: 250 }, placed.get("ad")!);
    const [r2] = stackAt(chipOnly("bc", "b", "c"), { x: 200, y: 250 }, { x: 400, y: 50 }, placed.get("bc")!);
    const apart = r1!.x + r1!.width <= r2!.x || r2!.x + r2!.width <= r1!.x || r1!.y + r1!.height <= r2!.y || r2!.y + r2!.height <= r1!.y;
    expect(apart).toBe(true);
  });

  it("a stack of flow labels moves as one, with the chip above the first", () => {
    const stack: LabelStack = { key: "s", from: "a", to: "b", arrows: [{ offset: -13, label: { width: 80, height: 16 } }, { offset: 13, label: { width: 80, height: 16 } }], chip: { width: 60, height: 22 }, chipAbove: 24 };
    const rects = stackAt(stack, { x: 0, y: 0 }, { x: 400, y: 0 }, 0.5);
    expect(rects).toHaveLength(3);
    expect(rects[1]!.y).toBeLessThan(rects[0]!.y);
  });
});
