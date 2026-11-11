/**
 * From a seam and a lens to the arrows the map draws (ADR 0017). Pure, so it is tested without
 * a DOM.
 *
 * Structure: one arrow per seam, from the side that depends to the side depended on. A flow
 * lens: one arrow per declared flow of that kind, in the flow's own direction, labelled with
 * what it carries; a seam with no flow of that kind keeps a faint plain arrow, so the picture
 * never loses a seam.
 */
import type { Flow, FlowKind } from "../../protocol.ts";

/** `plan` is not offered by the switch: it is on while an issue with a plan is selected. */
export type Lens = "structure" | FlowKind | "plan";

export const LENSES: { id: Exclude<Lens, "plan">; label: string }[] = [
  { id: "structure", label: "Structure" },
  { id: "data", label: "Data flow" },
  { id: "control", label: "Control flow" },
];

/** Pixels between two parallel arrows of one seam. */
export const ARROW_GAP = 26;

export interface Arrow {
  /** Unique on the map: the seam id for the seam's own arrow, `seam#kind-n` for a flow. */
  id: string;
  seam: string;
  /** What the arrow shows. `structure` is the seam itself. */
  kind: "structure" | FlowKind;
  /** The arrowhead sits at the seam's `from` end: the flow runs against the seam's direction. */
  reversed: boolean;
  /** What it carries, for a flow. */
  label: string | undefined;
  /** Sideways shift from the seam's own line, so parallel arrows do not overlap. */
  offset: number;
  dashed: boolean;
  /** No flow of the lens's kind is declared on this seam: drawn faint. */
  faint: boolean;
  /** A flow a plan proposes: drawn moving, so it reads as not there yet. */
  moving: boolean;
}

interface SeamLike {
  id: string;
  from: string;
  to: string;
  flows?: Flow[] | undefined;
}

export function arrowsFor(seam: SeamLike, lens: Lens): Arrow[] {
  const plain: Arrow = { id: seam.id, seam: seam.id, kind: "structure", reversed: false, label: undefined, offset: 0, dashed: false, faint: false, moving: false };
  if (lens === "structure") {
    return [plain];
  }
  // The plan lens shows everything the plan sends, both kinds at once.
  const flows = (seam.flows ?? []).filter((f) => lens === "plan" || f.kind === lens);
  if (flows.length === 0) {
    return [{ ...plain, faint: true }];
  }
  return flows.map((f, i) => ({
    id: `${seam.id}#${lens}-${i}`,
    seam: seam.id,
    kind: f.kind,
    reversed: f.from === seam.to,
    label: f.carries,
    offset: (i - (flows.length - 1) / 2) * ARROW_GAP,
    dashed: lens === "plan" || f.kind === "control",
    faint: false,
    moving: lens === "plan",
  }));
}

/** How many flows of each kind the map declares: what the legend and the lens buttons say. */
export function flowCounts(seams: readonly SeamLike[]): Record<FlowKind, number> {
  const out: Record<FlowKind, number> = { data: 0, control: 0 };
  for (const s of seams) {
    for (const f of s.flows ?? []) {
      out[f.kind] += 1;
    }
  }
  return out;
}
