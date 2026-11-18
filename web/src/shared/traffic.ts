import type { ObservedTraffic } from "../protocol.ts";

/**
 * A fresh report with nothing in it (issue #285): no call on a seam or off one, no service seen,
 * named or not, and nothing dismissed. The source answered, and there was nothing to compare
 * with the map; that is not the map matching.
 */
export function nothingObserved(t: Partial<ObservedTraffic> & Pick<ObservedTraffic, "state">): boolean {
  return (
    t.state === "fresh" &&
    !t.seams?.length &&
    !t.undeclared?.length &&
    !t.ghosts?.length &&
    !t.ghost_edges?.length &&
    !t.observed_parts?.length &&
    !t.unmapped_services &&
    !t.dismissed?.length
  );
}
