/**
 * The By repo grouping's shared arrangement (issue #144, ADR 0068), as pure functions.
 *
 * A part in a repo's box sits at a position relative to the box; a box sits on the canvas. Each
 * comes from this browser's hold (a drag, or a write not yet answered), else the team's stored
 * grouping, else ELK. A box is sized to fit its parts, so it grows when a part is dragged right
 * or down and shrinks when its parts move together. Nothing here reads or writes the flat map's
 * positions.
 */
import type { MapLayout } from "../../protocol.ts";
import { NODE_HEIGHT, NODE_WIDTH, type RepoGroup } from "./graph.ts";
import type { MoveChange, XY } from "./layout.ts";
import { mergeNodes } from "./nodes.ts";

/** Room inside a box around its parts; the top holds the repo's name. ELK lays out with the same. */
export const BOX_PAD = { left: 24, top: 48, right: 24, bottom: 24 } as const;

export interface Size {
  width: number;
  height: number;
}

export interface Rect extends XY, Size {}

/** ELK's grouping, or the arrangement to draw: boxes on the canvas, parts relative to their box. */
export interface Grouping {
  groups: RepoGroup[];
  parts: Map<string, XY & { parent?: string }>;
}

/** The repo a box node stands for, or undefined for a part's id (part ids hold no colon). */
export function boxRepo(id: string): string | undefined {
  return id.startsWith("repo:") ? id.slice("repo:".length) : undefined;
}

/** A part never sits left of or above its box's padding: dragged past the edge, it stays at it. */
export function clampInBox(p: XY): XY {
  return { x: Math.max(BOX_PAD.left, p.x), y: Math.max(BOX_PAD.top, p.y) };
}

/** Wide enough for the repo's name in the box's corner. */
function minWidth(repo: string): number {
  return Math.max(NODE_WIDTH + BOX_PAD.left + BOX_PAD.right, repo.length * 8 + 48);
}

/** The size of a box that holds these parts (positions relative to the box). */
export function fitBox(repo: string, parts: readonly Rect[]): Size {
  const right = Math.max(0, ...parts.map((p) => p.x + p.width));
  const bottom = Math.max(0, ...parts.map((p) => p.y + p.height));
  return {
    width: Math.max(minWidth(repo), right + BOX_PAD.right),
    height: Math.max(BOX_PAD.top + NODE_HEIGHT + BOX_PAD.bottom, bottom + BOX_PAD.bottom),
  };
}

export interface GroupedInput {
  elk: Grouping;
  layout: MapLayout | undefined;
  /** Held by this browser, by node id (`repo:owner/name` for a box). */
  held: ReadonlyMap<string, XY>;
  /** A part's drawn size, when React Flow has measured it. */
  sizeOf?: ((id: string) => Size | undefined) | undefined;
}

/** Where to draw every box and part, and each box's size. */
export function placeGrouped({ elk, layout, held, sizeOf }: GroupedInput): Grouping {
  const parts = new Map<string, XY & { parent?: string }>();
  for (const [id, auto] of elk.parts) {
    const p = held.get(id) ?? layout?.grouped?.[id] ?? { x: auto.x, y: auto.y };
    parts.set(id, auto.parent ? { ...clampInBox(p), parent: auto.parent } : { x: p.x, y: p.y });
  }
  const groups = elk.groups.map((g): RepoGroup => {
    const at = held.get(g.id) ?? layout?.boxes?.[g.repo] ?? { x: g.x, y: g.y };
    const inside = [...parts].filter(([, p]) => p.parent === g.id).map(([id, p]) => ({ ...p, ...(sizeOf?.(id) ?? { width: NODE_WIDTH, height: NODE_HEIGHT }) }));
    return { ...g, x: at.x, y: at.y, ...fitBox(g.repo, inside) };
  });
  return { groups, parts };
}

/** Where each node sits for the team (no hold): what a move is compared with. */
export function settledGrouped(elk: Grouping, layout: MapLayout | undefined): Map<string, XY> {
  const placed = placeGrouped({ elk, layout, held: new Map() });
  const out = new Map<string, XY>();
  for (const g of placed.groups) {
    out.set(g.id, { x: g.x, y: g.y });
  }
  for (const [id, p] of placed.parts) {
    out.set(id, { x: p.x, y: p.y });
  }
  return out;
}

function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/**
 * Where a node that came to rest at `to` stays: at `to`, unless it now lies on a box it does not
 * belong to (a part on another repo's box, a part with no repo on any box, a box on another
 * box). Then it goes back to `from`, where the drag began: moving code between repos is a map
 * draft (ADR 0018), not a drag.
 */
export function settle(id: string, to: XY, from: XY, placed: Grouping, sizeOf: (id: string) => Size): XY {
  const repo = boxRepo(id);
  const boxes = placed.groups;
  let rect: Rect;
  let own: string | undefined;
  if (repo !== undefined) {
    const g = boxes.find((b) => b.id === id);
    rect = { ...to, width: g?.width ?? 0, height: g?.height ?? 0 };
    own = id;
  } else {
    own = placed.parts.get(id)?.parent;
    const box = own ? boxes.find((b) => b.id === own) : undefined;
    rect = { x: to.x + (box?.x ?? 0), y: to.y + (box?.y ?? 0), ...sizeOf(id) };
  }
  return boxes.some((b) => b.id !== own && overlaps(rect, b)) ? from : to;
}

/**
 * What to write after a batch of node changes in the grouping: parts and boxes that came to rest
 * somewhere new, on whole pixels, as `grouped` and `boxes`. Never `positions`: the flat map is
 * not this view's to change. Null when nothing moved.
 */
export function groupedToSave(changes: readonly MoveChange[], shown: ReadonlyMap<string, XY>): MapLayout | null {
  const grouped: Record<string, XY> = {};
  const boxes: Record<string, XY> = {};
  for (const c of changes) {
    if (c.type !== "position" || !c.id || !c.position || c.dragging) {
      continue;
    }
    const to = { x: Math.round(c.position.x), y: Math.round(c.position.y) };
    const from = shown.get(c.id);
    if (from && Math.round(from.x) === to.x && Math.round(from.y) === to.y) {
      continue;
    }
    const repo = boxRepo(c.id);
    if (repo !== undefined) {
      boxes[repo] = to;
    } else {
      grouped[c.id] = to;
    }
  }
  if (!Object.keys(grouped).length && !Object.keys(boxes).length) {
    return null;
  }
  return { ...(Object.keys(grouped).length ? { grouped } : {}), ...(Object.keys(boxes).length ? { boxes } : {}) };
}

/** The node id a saved entry stands for: a part's own id, or `repo:<owner/name>` for a box. */
export function savedIds(l: MapLayout): string[] {
  return [...Object.keys(l.grouped ?? {}), ...Object.keys(l.boxes ?? {}).map((r) => `repo:${r}`)];
}

/** One node of React Flow's list, as far as box fitting reads it. */
interface Fittable {
  id: string;
  type?: string | undefined;
  position: XY;
  parentId?: string | undefined;
  measured?: { width?: number | undefined; height?: number | undefined } | undefined;
  style?: object | undefined;
}

/**
 * A refresh of the overview: `wanted` merged onto what React Flow drew (`mergeNodes`), then every
 * box fitted to its parts as measured. `placeGrouped` sizes a box from the default card height,
 * so without the refit a box was as tall as that until the next change, and two browsers could
 * show one box at two heights: a card taller than the default, at the bottom, left it short.
 */
export function refreshNodes<N extends Fittable & { data: unknown; parentId?: string }>(current: N[], wanted: N[]): N[] {
  return refitBoxes(mergeNodes(current, wanted));
}

/** Every box resized to fit its parts where they are now: while a part is dragged, its box follows. */
export function refitBoxes<N extends Fittable>(nodes: N[]): N[] {
  const children = new Map<string, Rect[]>();
  for (const n of nodes) {
    if (n.parentId) {
      const list = children.get(n.parentId) ?? [];
      list.push({ ...n.position, width: n.measured?.width ?? NODE_WIDTH, height: n.measured?.height ?? NODE_HEIGHT });
      children.set(n.parentId, list);
    }
  }
  return nodes.map((n) => {
    const repo = boxRepo(n.id);
    if (repo === undefined || n.type !== "repo") {
      return n;
    }
    const size = fitBox(repo, children.get(n.id) ?? []);
    const style = (n.style ?? {}) as { width?: unknown; height?: unknown };
    if (style.width === size.width && style.height === size.height) {
      return n;
    }
    return { ...n, style: { ...n.style, width: size.width, height: size.height } };
  });
}
