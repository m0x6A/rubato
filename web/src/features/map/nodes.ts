/**
 * Keep what React Flow has learned about a part across refreshes of the overview.
 *
 * React Flow measures each node once and hands the result back through `onNodesChange`
 * (`measured` on the node). It draws an edge only between measured nodes. Replacing every node
 * object on each refresh throws the measurements away, and the edges, with the seam chips on
 * them, vanish until the node happens to resize. So a refresh updates position and data on the
 * node that is already there.
 */
interface Mergeable {
  id: string;
  position: { x: number; y: number };
  data: unknown;
  /** The box a part is drawn in (the By repo grouping, issue #89); absent at the top level. */
  parentId?: string;
}

/** What React Flow writes onto a node it draws: kept across refreshes. Everything else is what is wanted. */
const LEARNED = ["measured", "selected", "dragging"] as const;

/**
 * `wanted` in its order, each keeping what React Flow learned about the node already there (its
 * measurement, its selection, whether it is being dragged). Everything else comes from what is
 * wanted, so a box's new size, a part's box and whether a node can be dragged follow the view
 * (issue #144: a box kept its first size).
 */
export function mergeNodes<N extends Mergeable>(current: N[], wanted: N[]): N[] {
  const byId = new Map(current.map((n) => [n.id, n]));
  return wanted.map((w) => {
    const have = byId.get(w.id) as Record<string, unknown> | undefined;
    if (!have) {
      return w;
    }
    const out: Record<string, unknown> = { ...(w as unknown as Record<string, unknown>) };
    for (const k of LEARNED) {
      if (have[k] !== undefined && out[k] === undefined) {
        out[k] = have[k];
      }
    }
    return out as N;
  });
}
