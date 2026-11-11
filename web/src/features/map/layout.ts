/**
 * The rules of the shared map layout (ADR 0016), as pure functions.
 *
 * A part sits where the team last put it; a part nobody has placed is placed by ELK. While this
 * browser drags a part, or waits for the answer to its own write, it holds the part where the
 * person put it, so an overview that arrives meanwhile cannot pull it from under the pointer.
 */
import type { MapLayout } from "../../protocol.ts";

export interface XY {
  x: number;
  y: number;
}

/** Where to draw a part: held here, else stored for the team, else the automatic layout. */
export function placePart(p: { auto: XY | undefined; stored: XY | undefined; held: XY | undefined }): XY {
  return p.held ?? p.stored ?? p.auto ?? { x: 0, y: 0 };
}

/** The part of a React Flow node change this module reads. */
export interface MoveChange {
  type: string;
  id?: string;
  position?: XY | undefined;
  dragging?: boolean | undefined;
}

/**
 * What to write after a batch of node changes: the parts that came to rest somewhere new, on
 * whole pixels. A drag ends with `dragging: false`; an arrow-key move never sets `dragging`.
 * Null when nothing moved, so no request is made.
 */
export function positionsToSave(changes: readonly MoveChange[], shown: ReadonlyMap<string, XY>): MapLayout | null {
  const positions: Record<string, XY> = {};
  for (const c of changes) {
    if (c.type !== "position" || !c.id || !c.position || c.dragging) {
      continue;
    }
    const to = { x: Math.round(c.position.x), y: Math.round(c.position.y) };
    const from = shown.get(c.id);
    if (!from || Math.round(from.x) !== to.x || Math.round(from.y) !== to.y) {
      positions[c.id] = to;
    }
  }
  return Object.keys(positions).length ? { positions } : null;
}
