import type { MergeRequest, MergeResponse, PullDetail } from "../../protocol.ts";
import { ApiError, failureText, request } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

/** A pull request's view and the merge, both map-scoped (ADR 0049). */
export const pullsApi = {
  view: (repo: string, number: number) => request<PullDetail>("GET", scoped(`/api/pulls/${repo.split("/").map(encodeURIComponent).join("/")}/${number}`)),
  merge: (req: MergeRequest) => request<MergeResponse>("POST", scoped("/api/pulls/merge"), req),
};

/** Why *Merge* is off or was refused, in words (design 82 §3, in the order the server checks). */
export const WHY_NOT: Record<string, string> = {
  not_open: "It is not open.",
  fork: "It comes from a fork; Rubato merges only branches of the repo itself.",
  draft: "It is a draft.",
  head_changed: "Someone pushed since you looked. Look again, then merge.",
  checks_not_green: "Checks are not green: failing, still running, or not started.",
  not_mergeable: "GitHub reports conflicts, or is still checking whether it can merge. Look again in a moment.",
  plan_not_approved: "The issue's plan is not approved as it is at this head.",
  needs_github_authorization: "Rubato cannot act as you yet: log in again to authorise it.",
  no_push_permission: "You have no push permission on this repo.",
  method_not_allowed: "The repo does not allow that merge method.",
  github_refused: "GitHub refused the merge.",
  github_failed: "Rubato could not read the pull request from GitHub; nothing was sent.",
  unknown_repo: "This repo is not in the map, or the App does not cover it.",
};

/** Why the plan gate is closed (`PlanGate.why`). */
export const PLAN_WHY: Record<string, string> = {
  no_approval: "no counted approval",
  not_named: "the approval names no version",
  missing_at_head: "the plan file is not at this head",
  changed_at_head: "the plan at this head is not the version approved",
  does_not_fit: "the plan at this head does not fit the map",
  unreadable: "the plan at this head could not be read",
  record_missing: "the plan at this head carries no approval record",
  record_differs: "the approval record at this head is not Rubato's",
};

/** The refusal of a failed call: its reason and GitHub's message, when the server sent them. */
export function refusalOf(e: unknown): { reason: string | null; message: string | null; text: string } {
  if (!(e instanceof ApiError)) {
    return { reason: null, message: null, text: failureText(e) };
  }
  try {
    const v = JSON.parse(e.message) as { reason?: unknown; message?: unknown };
    const reason = typeof v.reason === "string" ? v.reason : null;
    const message = typeof v.message === "string" ? v.message : null;
    const words = reason ? (WHY_NOT[reason] ?? reason) : e.message;
    return { reason, message, text: message ? `${words} ${message}` : words };
  } catch {
    return { reason: null, message: null, text: e.message };
  }
}
