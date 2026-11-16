/**
 * The map as people see it (ADR 0077): every reader that draws, offers or suggests a part goes
 * through `liveMap`, so a retired node or seam is drawn and offered nowhere. A retired id is
 * still declared, so a lookup by id (a pin on it, the territory's taken ids) uses the full map.
 */
import type { SystemMap } from "../protocol.ts";

export function liveMap(map: SystemMap): SystemMap {
  const nodes = map.nodes.filter((n) => n.retired == null);
  if (map.seams === undefined) {
    return nodes.length === map.nodes.length ? map : { ...map, nodes };
  }
  const seams = map.seams.filter((s) => s.retired == null);
  if (nodes.length === map.nodes.length && seams.length === map.seams.length) {
    return map;
  }
  return { ...map, nodes, seams };
}

/** Whether `id` names a retired node or seam of `map`. */
export function isRetired(map: SystemMap, id: string): boolean {
  return map.nodes.some((n) => n.id === id && n.retired != null) || (map.seams ?? []).some((s) => s.id === id && s.retired != null);
}
