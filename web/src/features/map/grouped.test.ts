import { describe, expect, it } from "vitest";
import { NODE_HEIGHT, NODE_WIDTH } from "./graph.ts";
import { BOX_PAD, type Grouping, clampInBox, fitBox, groupedToSave, placeGrouped, refitBoxes, refreshNodes, savedIds, settle, settledGrouped } from "./grouped.ts";

const at = (x: number, y: number) => ({ x, y });
const size = () => ({ width: NODE_WIDTH, height: NODE_HEIGHT });

/** Two repos side by side and one part with no repo, as ELK would give them. */
const elk = (): Grouping => ({
  groups: [
    { id: "repo:acme/auth", repo: "acme/auth", x: 0, y: 0, width: 600, height: 400 },
    { id: "repo:acme/shop", repo: "acme/shop", x: 800, y: 0, width: 300, height: 200 },
  ],
  parts: new Map([
    ["auth-api", { x: 24, y: 48, parent: "repo:acme/auth" }],
    ["auth-contract", { x: 300, y: 48, parent: "repo:acme/auth" }],
    ["shop-web", { x: 24, y: 48, parent: "repo:acme/shop" }],
    ["payments", { x: 1200, y: 300 }],
  ]),
});

describe("the By repo arrangement (issue #144, ADR 0068)", () => {
  it("a part sits where it is held, else where the team put it, else where ELK put it", () => {
    const layout = { grouped: { "auth-api": at(100, 60) }, boxes: { "acme/shop": at(900, 500) } };
    const placed = placeGrouped({ elk: elk(), layout, held: new Map([["auth-contract", at(400, 90)]]) });
    expect(placed.parts.get("auth-api")).toEqual({ x: 100, y: 60, parent: "repo:acme/auth" });
    expect(placed.parts.get("auth-contract")).toEqual({ x: 400, y: 90, parent: "repo:acme/auth" });
    expect(placed.parts.get("shop-web")).toEqual({ x: 24, y: 48, parent: "repo:acme/shop" });
    expect(placed.groups.find((g) => g.repo === "acme/shop")).toMatchObject({ x: 900, y: 500 });
    expect(placed.groups.find((g) => g.repo === "acme/auth")).toMatchObject({ x: 0, y: 0 });
    // A part with no repo has a position on the canvas, from the same three sources.
    expect(placeGrouped({ elk: elk(), layout: { grouped: { payments: at(5, 6) } }, held: new Map() }).parts.get("payments")).toEqual({ x: 5, y: 6 });
  });

  it("the flat map's positions are never read", () => {
    const placed = placeGrouped({ elk: elk(), layout: { positions: { "auth-api": at(9999, 9999) } }, held: new Map() });
    expect(placed.parts.get("auth-api")).toEqual({ x: 24, y: 48, parent: "repo:acme/auth" });
  });

  it("a box fits its parts: it grows with them and shrinks when they move together, never narrower than its name", () => {
    const one = [{ ...at(BOX_PAD.left, BOX_PAD.top), ...size() }];
    const small = fitBox("acme/auth", one);
    expect(small).toEqual({ width: BOX_PAD.left + NODE_WIDTH + BOX_PAD.right, height: BOX_PAD.top + NODE_HEIGHT + BOX_PAD.bottom });
    const grown = fitBox("acme/auth", [...one, { ...at(500, 300), ...size() }]);
    expect(grown).toEqual({ width: 500 + NODE_WIDTH + BOX_PAD.right, height: 300 + NODE_HEIGHT + BOX_PAD.bottom });
    // ELK's box was 600 × 400; placed, it is cut to its parts.
    const placed = placeGrouped({ elk: elk(), layout: undefined, held: new Map() });
    expect(placed.groups[0]).toMatchObject({ width: 300 + NODE_WIDTH + BOX_PAD.right, height: 48 + NODE_HEIGHT + BOX_PAD.bottom });
    const long = "a-very-long-organisation-name/and-an-even-longer-repository-name";
    expect(fitBox(long, one).width).toBeGreaterThan(long.length * 8);
  });

  it("a part dragged past its box's left or top edge stays at the edge", () => {
    expect(clampInBox(at(-50, 10))).toEqual(at(BOX_PAD.left, BOX_PAD.top));
    expect(clampInBox(at(100, 200))).toEqual(at(100, 200));
    // A stored position from before is clamped too.
    const placed = placeGrouped({ elk: elk(), layout: { grouped: { "auth-api": at(-10, -10) } }, held: new Map() });
    expect(placed.parts.get("auth-api")).toMatchObject(at(BOX_PAD.left, BOX_PAD.top));
  });

  it("a part or a box dropped on another repo's box goes back to where the drag began", () => {
    const placed = placeGrouped({ elk: elk(), layout: undefined, held: new Map() });
    const shop = placed.groups.find((g) => g.repo === "acme/shop")!;
    // auth-api, relative to its box at the origin, dropped onto acme/shop's box.
    expect(settle("auth-api", at(shop.x + 10, 60), at(24, 48), placed, size)).toEqual(at(24, 48));
    // Inside its own box (it grows to make room): stays.
    expect(settle("auth-api", at(200, 300), at(24, 48), placed, size)).toEqual(at(200, 300));
    // A part with no repo dropped on a box goes back; on empty canvas it stays.
    expect(settle("payments", at(shop.x + 5, shop.y + 5), at(1200, 300), placed, size)).toEqual(at(1200, 300));
    expect(settle("payments", at(1500, 900), at(1200, 300), placed, size)).toEqual(at(1500, 900));
    // A box dropped on the other box goes back; next to it, stays.
    expect(settle("repo:acme/auth", at(shop.x - 50, 0), at(0, 0), placed, size)).toEqual(at(0, 0));
    expect(settle("repo:acme/auth", at(0, 900), at(0, 0), placed, size)).toEqual(at(0, 900));
  });

  it("a move is written as grouped or boxes, never as the flat map's positions", () => {
    const shown = settledGrouped(elk(), undefined);
    expect(shown.get("repo:acme/shop")).toEqual(at(800, 0));
    const out = groupedToSave(
      [
        { type: "position", id: "auth-api", position: at(100.4, 60.6), dragging: false },
        { type: "position", id: "repo:acme/shop", position: at(820, 10), dragging: false },
        // Still dragging, or not moved: nothing to write.
        { type: "position", id: "auth-contract", position: at(1, 1), dragging: true },
        { type: "position", id: "shop-web", position: at(24, 48), dragging: false },
        { type: "dimensions", id: "payments" },
      ],
      shown,
    );
    expect(out).toEqual({ grouped: { "auth-api": at(100, 61) }, boxes: { "acme/shop": at(820, 10) } });
    expect(out).not.toHaveProperty("positions");
    expect(savedIds(out!)).toEqual(["auth-api", "repo:acme/shop"]);
    expect(groupedToSave([{ type: "position", id: "shop-web", position: at(24, 48) }], shown)).toBeNull();
  });

  it("while a part is dragged its box follows it", () => {
    const nodes = [
      { id: "repo:acme/auth", type: "repo", position: at(0, 0), style: { width: 10, height: 10 } },
      { id: "auth-api", type: "map", position: at(400, 300), parentId: "repo:acme/auth", measured: { width: 220, height: 120 } },
      { id: "payments", type: "map", position: at(0, 0) },
    ];
    const fitted = refitBoxes(nodes);
    expect(fitted[0]?.style).toEqual({ width: 400 + 220 + BOX_PAD.right, height: 300 + 120 + BOX_PAD.bottom });
    expect(fitted[2]).toBe(nodes[2]);
    // Unchanged when it already fits: the same object, so React Flow is not told twice.
    expect(refitBoxes(fitted)[0]).toBe(fitted[0]);
  });

  it("a refresh of the overview keeps each box fitted to its measured parts, not to the default card height", () => {
    // The box as placeGrouped sizes it, from NODE_HEIGHT; the part as React Flow measured it, taller.
    type Drawn = { id: string; type: string; position: { x: number; y: number }; data: unknown; parentId?: string; style?: object; measured?: { width: number; height: number } };
    const wanted: Drawn[] = [
      { id: "repo:acme/auth", type: "repo", position: at(0, 0), data: {}, style: { width: 400 + NODE_WIDTH + BOX_PAD.right, height: 300 + NODE_HEIGHT + BOX_PAD.bottom } },
      { id: "auth-api", type: "map", position: at(400, 300), parentId: "repo:acme/auth", data: {} },
    ];
    const current = [wanted[0]!, { ...wanted[1]!, measured: { width: NODE_WIDTH, height: NODE_HEIGHT + 8 } }];
    const next = refreshNodes(current, wanted);
    expect(next[1]?.measured).toEqual({ width: NODE_WIDTH, height: NODE_HEIGHT + 8 });
    expect(next[0]?.style).toEqual({ width: 400 + NODE_WIDTH + BOX_PAD.right, height: 300 + NODE_HEIGHT + 8 + BOX_PAD.bottom });
  });
});
