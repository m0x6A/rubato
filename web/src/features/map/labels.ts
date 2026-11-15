/**
 * Where along its arrow each seam's labels sit in the By repo grouping (issue #144), as pure
 * functions. React Flow puts a label at the middle of its curve; in the grouping that middle often
 * lies on a part card (a seam that leaves a box crosses the box's last column) or on another
 * label (two curves that cross). So each seam's labels (its chip and the flow labels below it, or
 * a bundle's badge) try points along the curve, from the middle outwards, and take the first that
 * touches no part card and no label placed before them. The flat map keeps its middles.
 */
import type { XY } from "./layout.ts";

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** React Flow's control offset for a bezier from a right handle to a left one (`getBezierPath`, curvature 0.25). */
function controlOffset(distance: number): number {
  return distance >= 0 ? 0.5 * distance : 0.25 * 25 * Math.sqrt(-distance);
}

/** The point at `t` of the curve React Flow draws from a right handle at `s` to a left handle at `e`. */
export function bezierAt(s: XY, e: XY, t: number): XY {
  const c1 = { x: s.x + controlOffset(e.x - s.x), y: s.y };
  const c2 = { x: e.x - controlOffset(e.x - s.x), y: e.y };
  const u = 1 - t;
  const a = u * u * u;
  const b = 3 * u * u * t;
  const c = 3 * u * t * t;
  const d = t * t * t;
  return { x: a * s.x + b * c1.x + c * c2.x + d * e.x, y: a * s.y + b * c1.y + c * c2.y + d * e.y };
}

/** Where along the curve a label may go, best first: the middle, then outwards. */
export const CANDIDATES = [0.5, 0.42, 0.58, 0.34, 0.66, 0.26, 0.74, 0.18, 0.82, 0.1, 0.9] as const;

/** One seam's labels, or one bundle's badge: what must stay clear of cards and other labels. */
export interface LabelStack {
  key: string;
  from: string;
  to: string;
  /** Each arrow's sideways shift and its label's size (none for a plain seam arrow). */
  arrows: { offset: number; label: { width: number; height: number } | null }[];
  /** The chip (or badge) on the first arrow, raised by `chipAbove` when that arrow has a label. */
  chip: { width: number; height: number };
  chipAbove: number;
}

function centred(p: XY, w: number, h: number): Rect {
  return { x: p.x - w / 2, y: p.y - h / 2, width: w, height: h };
}

/** The rectangles of a stack's labels when they sit at `t`. */
export function stackAt(s: LabelStack, from: XY, to: XY, t: number): Rect[] {
  const out: Rect[] = [];
  s.arrows.forEach((a, i) => {
    const p = bezierAt({ x: from.x, y: from.y + a.offset }, { x: to.x, y: to.y + a.offset }, t);
    if (a.label) {
      out.push(centred(p, a.label.width, a.label.height));
    }
    if (i === 0) {
      out.push(centred({ x: p.x, y: p.y - (a.label ? s.chipAbove : 0) }, s.chip.width, s.chip.height));
    }
  });
  return out;
}

function overlap(a: Rect, b: Rect): number {
  const w = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}

/** A little room around every rectangle: handles sit a few pixels off a card's edge, and text is measured, not known. */
const MARGIN = 6;

function grow(r: Rect): Rect {
  return { x: r.x - MARGIN, y: r.y - MARGIN, width: r.width + 2 * MARGIN, height: r.height + 2 * MARGIN };
}

/**
 * The `t` for each stack, in the given order: the first candidate whose rectangles touch no card
 * and no label already placed; when none is clear, the one that overlaps least. Stacks whose ends
 * have no card (not measured yet) keep the middle.
 */
export function placeLabels(stacks: readonly LabelStack[], cards: ReadonlyMap<string, Rect>): Map<string, number> {
  const out = new Map<string, number>();
  const taken: Rect[] = [...cards.values()].map(grow);
  for (const s of stacks) {
    const a = cards.get(s.from);
    const b = cards.get(s.to);
    if (!a || !b) {
      out.set(s.key, 0.5);
      continue;
    }
    // A right handle on the source, a left handle on the target: React Flow's own ends.
    const from = { x: a.x + a.width, y: a.y + a.height / 2 };
    const to = { x: b.x, y: b.y + b.height / 2 };
    let best: { t: number; cost: number; rects: Rect[] } | null = null;
    for (const t of CANDIDATES) {
      const rects = stackAt(s, from, to, t);
      const cost = rects.reduce((sum, r) => sum + taken.reduce((n, k) => n + overlap(r, k), 0), 0);
      if (!best || cost < best.cost) {
        best = { t, cost, rects };
      }
      if (cost === 0) {
        break;
      }
    }
    out.set(s.key, best!.t);
    taken.push(...best!.rects.map(grow));
  }
  return out;
}

/** Roughly how wide a line of monospace text is: the label is measured by the browser, this is for placing it. */
export function textWidth(chars: number, px: number, padding: number): number {
  return Math.ceil(chars * px * 0.62 + padding);
}
