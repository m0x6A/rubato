/** Formatting helpers every feature uses. Pure. */
import type { ApprovalState, IssueRef, ProgressState, ReviewRecord } from "../protocol.ts";

export function issueKey(i: IssueRef): string {
  return `${i.repo}#${i.number}`;
}

export function parseIssueKey(s: string): IssueRef | null {
  const m = /^([^\s/#]+\/[^\s/#]+)#(\d+)$/.exec(s.trim());
  return m ? { repo: m[1] as string, number: Number(m[2]) } : null;
}

export const STATE_LABEL: Record<ProgressState, string> = {
  planned: "Planned",
  started: "Started",
  designing: "Designing",
  designed: "Designed",
  building: "Building",
  blocked: "Blocked",
  in_review: "In review",
  done: "Done",
  unknown: "Unknown",
};

export function relative(iso: string, now = Date.now()): string {
  const secs = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (secs < 60) {
    return `${secs}s ago`;
  }
  if (secs < 3600) {
    return `${Math.round(secs / 60)}m ago`;
  }
  if (secs < 86400) {
    return `${Math.round(secs / 3600)}h ago`;
  }
  return `${Math.round(secs / 86400)}d ago`;
}

/** The short label of an issue's approval (ADR 0027), as its badge shows it. */
export function approvalLabel(a: ApprovalState): string {
  switch (a.version) {
    case "current":
      return "approved";
    case "earlier":
      return "approved an earlier version";
    case "not_named":
      return "approved, version not named";
    case "no_plan":
      return a.record.branch ? `approved, no plan on ${a.record.branch}` : "approved, no plan to compare";
  }
}

/** The badge class of an approval: green when it names the plan as it is now. */
export function approvalClass(a: ApprovalState): string {
  return a.version === "current" ? "approved" : "approval-stale";
}

/** One line for a review (ADR 0027): `reviewed (full, passed after fixes)`. */
export function reviewLabel(r: ReviewRecord): string {
  return `reviewed (${r.track}, ${r.outcome})`;
}
