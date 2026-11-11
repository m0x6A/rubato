/**
 * The plan lens (design S11): when an issue with a plan is selected, the map shows that plan
 * and nothing else. Parts the plan names are painted with its status, every other part steps
 * back, and the plan's flows are drawn on their seams. Pure, so it is tested without a DOM.
 */
import type { Flow, PartStatus, Plan } from "../../protocol.ts";

export interface PlanLens {
  /** How the plan paints each part it names. */
  parts: Map<string, PartStatus>;
  /** The plan's flows by seam, in the form a seam's own flows have. */
  flows: Map<string, Flow[]>;
  /** Parts that stay lit: named by the plan, or at an end of one of its flows. */
  litParts: Set<string>;
  /** Seams that stay lit: named in affected parts, in an O-row, or carrying a flow. */
  litSeams: Set<string>;
  /** Flows that could not be drawn: their seam is not on the map. */
  unplaced: number;
}

export function planLens(plan: Plan, seams: readonly { id: string; from: string; to: string }[]): PlanLens {
  const known = new Map(seams.map((s) => [s.id, s]));
  const parts = new Map<string, PartStatus>();
  const litSeams = new Set<string>();
  for (const p of plan.affected_parts) {
    parts.set(p.node, p.status);
    for (const s of p.seams ?? []) {
      litSeams.add(s);
    }
  }
  for (const row of plan.o_rows ?? []) {
    litSeams.add(row.seam);
  }
  const litParts = new Set(parts.keys());
  const flows = new Map<string, Flow[]>();
  let unplaced = 0;
  for (const f of plan.flows ?? []) {
    const seam = known.get(f.seam);
    // Drawn only where it can be drawn truthfully: on a seam of the map that joins its two parts.
    if (!seam || !((f.from === seam.from && f.to === seam.to) || (f.from === seam.to && f.to === seam.from))) {
      unplaced += 1;
      continue;
    }
    flows.set(f.seam, [...(flows.get(f.seam) ?? []), { kind: f.kind, from: f.from, to: f.to, carries: f.carries }]);
    litSeams.add(f.seam);
    litParts.add(f.from);
    litParts.add(f.to);
  }
  return { parts, flows, litParts, litSeams, unplaced };
}
