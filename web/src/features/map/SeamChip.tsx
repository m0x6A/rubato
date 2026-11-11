import { type OnPin, useIssueDrop } from "./drop.ts";
import type { GraphEdge } from "./graph.ts";

interface Props {
  seam: GraphEdge;
  selected: boolean;
  onSelect: (id: string) => void;
  /** Called when an issue row is dropped on the chip. Without it the chip takes no drops. */
  onPin?: OnPin | undefined;
}

/**
 * A seam's label on the map: a button that selects the seam (click, Enter or Space) and the drop
 * target for pinning an issue to it, since React Flow edges take no drops (design S3, S4).
 * Renders without React Flow, so it is tested on its own.
 */
export function SeamChip({ seam, selected, onSelect, onPin }: Props) {
  const { over, handlers } = useIssueDrop<HTMLButtonElement>({ kind: "seam", id: seam.id }, onPin);
  const plans = seam.openPlans.length;
  const classes = ["seam-chip", "nodrag", "nopan", seam.collision ? "collision" : plans ? "busy" : "", selected ? "selected" : "", over ? "drop-over" : ""].filter(Boolean).join(" ");
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
      {plans > 0 && ` · ${plans} plan${plans === 1 ? "" : "s"}`}
      {pinned > 0 && ` · ${pinned} issue${pinned === 1 ? "" : "s"}`}
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
