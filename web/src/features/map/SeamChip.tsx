import { type OnPin, useIssueDrop } from "./drop.ts";
import type { DraftMarkLike, GraphEdge } from "./graph.ts";

interface Props {
  seam: GraphEdge;
  selected: boolean;
  onSelect: (id: string) => void;
  /** Called when an issue row is dropped on the chip. Without it the chip takes no drops. */
  onPin?: OnPin | undefined;
  /** The shared draft adds, changes or removes this seam. */
  draft?: DraftMarkLike | undefined;
}

/**
 * A seam's label on the map: a button that selects the seam (click, Enter or Space) and the drop
 * target for pinning an issue to it, since React Flow edges take no drops (design S3, S4).
 * Renders without React Flow, so it is tested on its own.
 */
export function SeamChip({ seam, selected, onSelect, onPin, draft }: Props) {
  const { over, handlers } = useIssueDrop<HTMLButtonElement>({ kind: "seam", id: seam.id }, draft?.state === "added" ? undefined : onPin);
  const plans = seam.openPlans.length;
  const classes = ["seam-chip", "nodrag", "nopan", seam.collision || seam.findings.length ? "collision" : plans ? "busy" : "", selected ? "selected" : "", draft ? `draft draft-${draft.state}` : "", over ? "drop-over" : ""].filter(Boolean).join(" ");
  const pinned = seam.issues.length;
  return (
    <button
      type="button"
      className={classes}
      data-testid={`seam-${seam.id}`}
      aria-pressed={selected}
      title={`Seam ${seam.from} → ${seam.to}, owned by ${seam.owner}${seam.carries ? `: ${seam.carries}` : ""}`}
      onClick={(e) => {
        e.stopPropagation();
        onSelect(seam.id);
      }}
      {...handlers}
    >
      {seam.id}
      {draft && ` · ${draft.state === "added" ? "draft" : draft.state === "changed" ? "changed in draft" : "removed in draft"} · ${draft.author}`}
      {plans > 0 && ` · ${plans} plan${plans === 1 ? "" : "s"}`}
      {pinned > 0 && ` · ${pinned} issue${pinned === 1 ? "" : "s"}`}
      {seam.findings.length > 0 && (
        <>
          {" "}
          <span aria-hidden="true">⚠</span>
          <span className="sr-only">
            {" "}
            {seam.findings.length} finding{seam.findings.length === 1 ? "" : "s"}: a plan sends something over this seam that it does not join
          </span>
        </>
      )}
      {seam.collision && (
        <>
          {" "}
          <span aria-hidden="true">⚠</span>
          <span className="sr-only"> two open plans touch this seam</span>
        </>
      )}
    </button>
  );
}
