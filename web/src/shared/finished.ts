import type { IssueSummary, Overview } from "../protocol.ts";

/** Whether an issue is finished (#265): merged, or closed on GitHub with no agent working. The
 * server decides (`is_finished`) and sends `finished`; nothing here derives it again. */
export const isFinished = (i: IssueSummary): boolean => i.finished === true;

/** The issues the default views show: open on GitHub and not merged. */
export function openIssues(ov: Overview): IssueSummary[] {
  return ov.issues.filter((i) => !isFinished(i));
}

/** When an issue finished, as near as the overview knows: the merge, else its last signal. */
function finishedAt(i: IssueSummary): string {
  return i.pull_request?.merged_at ?? i.progress.last_seen;
}

/** The issues the Done views show, newest first. */
export function finishedIssues(ov: Overview): IssueSummary[] {
  return ov.issues.filter(isFinished).sort((a, b) => finishedAt(b).localeCompare(finishedAt(a)) || b.issue.number - a.issue.number);
}

/** Closed on GitHub without its pull request merging: the Done views mark it *closed*. */
export const closedUnmerged = (i: IssueSummary): boolean => i.closed === true && i.progress.state !== "done";
