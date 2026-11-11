/**
 * The payload of an issue row being dragged (native HTML drag and drop, design D9). It lives in
 * `shared/` because the issue list starts the drag and the map ends it, and neither may import the
 * other's inner files.
 */
import type { IssueRef } from "../protocol.ts";

export const ISSUE_MIME = "application/x-rubato-issue";

/** The part of a drag event these helpers need; React's and the DOM's events both fit. */
interface HasDataTransfer {
  dataTransfer: DataTransfer | null;
}

/** Call from `dragstart` on an issue row. */
export function setIssueDrag(e: HasDataTransfer, issue: IssueRef): void {
  if (!e.dataTransfer) {
    return;
  }
  e.dataTransfer.setData(ISSUE_MIME, JSON.stringify({ repo: issue.repo, number: issue.number }));
  e.dataTransfer.setData("text/plain", `${issue.repo}#${issue.number}`);
  e.dataTransfer.effectAllowed = "copy";
}

/** True while an issue is being dragged. Browsers hide the data until the drop, but not the type. */
export function hasIssueDrag(e: HasDataTransfer): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes(ISSUE_MIME);
}

/** Call from `drop`. Null when the drag is not an issue or its payload is not one. */
export function getIssueDrag(e: HasDataTransfer): IssueRef | null {
  const raw = e.dataTransfer?.getData(ISSUE_MIME);
  if (!raw) {
    return null;
  }
  try {
    const v = JSON.parse(raw) as Partial<IssueRef> | null;
    if (v && typeof v.repo === "string" && typeof v.number === "number" && Number.isInteger(v.number)) {
      return { repo: v.repo, number: v.number };
    }
  } catch {
    // not ours
  }
  return null;
}
