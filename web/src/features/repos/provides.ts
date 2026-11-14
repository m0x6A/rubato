/**
 * What a repo provides to the whole application (issue #89), derived from the map and nothing
 * else: the parts whose code lives in it, the seams it owns, the seams it uses that another part
 * owns, and the journeys its parts take part in. Pure, so it is tested without a DOM.
 */
import type { MapNode, Scenario, Seam, SystemMap } from "../../protocol.ts";

export interface RepoFeatures {
  parts: MapNode[];
  /** Seams owned by one of its parts: the contracts this repo answers for. */
  owns: Seam[];
  /** Seams that touch one of its parts but are owned elsewhere: what it depends on. */
  uses: Seam[];
  /** Journeys with a step from or to one of its parts (ADR 0024). */
  scenarios: Scenario[];
}

const same = (a: string | null | undefined, b: string) => (a ?? "").toLowerCase() === b.toLowerCase();

export function repoFeatures(map: SystemMap, repo: string): RepoFeatures {
  const parts = map.nodes.filter((n) => same(n.repo, repo));
  const ids = new Set(parts.map((p) => p.id));
  const seams = map.seams ?? [];
  return {
    parts,
    owns: seams.filter((s) => ids.has(s.owner)),
    uses: seams.filter((s) => !ids.has(s.owner) && (ids.has(s.from) || ids.has(s.to))),
    scenarios: (map.scenarios ?? []).filter((sc) => sc.steps.some((st) => ids.has(st.from) || ids.has(st.to))),
  };
}
