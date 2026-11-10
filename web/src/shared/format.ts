/** Formatting helpers every feature uses. Pure. */
import type { IssueRef, ProgressState } from "../protocol.ts";

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
