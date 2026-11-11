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
}

/** `wanted` in its order, each taking over what `current` holds for the same id. */
export function mergeNodes<N extends Mergeable>(current: N[], wanted: N[]): N[] {
  const byId = new Map(current.map((n) => [n.id, n]));
  return wanted.map((w) => {
    const have = byId.get(w.id);
    return have ? { ...have, position: w.position, data: w.data } : w;
  });
}
