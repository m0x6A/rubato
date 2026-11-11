import { type DragEvent, useState } from "react";
import type { IssueRef, MapRef } from "../../protocol.ts";
import { getIssueDrag, hasIssueDrag } from "../../shared/dnd.ts";

export type OnPin = (issue: IssueRef, target: MapRef) => void;

/**
 * Makes an element a drop target for a dragged issue row (design S3, D9): `over` is true while an
 * issue is above it, and a drop calls `onPin` with the target. Other drags are left alone.
 */
export function useIssueDrop<E extends HTMLElement>(target: MapRef, onPin: OnPin | undefined) {
  const [over, setOver] = useState(false);
  const accept = (e: DragEvent<E>) => {
    if (!onPin || !hasIssueDrag(e)) {
      return;
    }
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setOver(true);
  };
  return {
    over,
    handlers: {
      onDragEnter: accept,
      onDragOver: accept,
      onDragLeave: (e: DragEvent<E>) => {
        // Moving onto a child also fires dragleave; only leaving the element ends the highlight.
        if (!(e.relatedTarget instanceof Node) || !e.currentTarget.contains(e.relatedTarget)) {
          setOver(false);
        }
      },
      onDrop: (e: DragEvent<E>) => {
        setOver(false);
        const issue = getIssueDrag(e);
        if (!onPin || !issue) {
          return;
        }
        e.preventDefault();
        onPin(issue, target);
      },
    },
  };
}
