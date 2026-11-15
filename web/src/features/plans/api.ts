import type { ApproveRequest, ApproveResponse, DraftApproveRequest, DraftApproveResponse, DraftRevision, DraftView } from "../../protocol.ts";
import { ApiError, failureText, request } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

/** Why an approval was refused, from the server's JSON answer (ADR 0030). */
export function refusalReason(e: unknown): string | null {
  if (!(e instanceof ApiError)) {
    return null;
  }
  try {
    const v = JSON.parse(e.message) as { reason?: unknown };
    return typeof v.reason === "string" ? v.reason : null;
  } catch {
    return null;
  }
}

export const plansApi = {
  approve: (req: ApproveRequest) => request<ApproveResponse>("POST", scoped("/api/plans/approve"), req),
};

/** What a refused approval tells the person. */
export const REFUSAL_TEXT: Record<string, string> = {
  plan_changed: "The plan changed since you saw it. Look again, then accept.",
  plan_does_not_fit: "The plan does not fit the map, so it cannot be accepted.",
  needs_github_authorization: "Rubato cannot comment as you yet: log in again to authorise it.",
  github_refused: "GitHub refused the comment. Check that you can comment on the issue.",
  no_plan: "There is no plan file for this issue.",
  approve_in_rubato: "Plans are approved in Rubato on this server: approve the draft instead.",
};

/** A plan draft and its revisions (ADR 0053): team routes, never `scoped()`. */
export const planDraftsApi = {
  draft: (draftId: string) => request<DraftView>("GET", `/api/drafts/${encodeURIComponent(draftId)}`),
  revision: (draftId: string, rev: number) => request<DraftRevision>("GET", `/api/drafts/${encodeURIComponent(draftId)}/revisions/${rev}`),
  /** Approve one revision as yourself (ADR 0054). Map-scoped: the fit check reads the workspace. */
  approve: (req: DraftApproveRequest) => request<DraftApproveResponse>("POST", scoped("/api/plans/drafts/approve"), req),
};

/** Why an approval in Rubato was refused, in words; the code is shown beside them. */
export const DRAFT_APPROVE_REFUSAL: Record<string, string> = {
  cutover_unset: "Approval in Rubato is not switched on on this server yet",
  no_draft: "There is no draft on that branch",
  not_latest: "A newer revision arrived since you looked; read it, then approve",
  not_settled: "The session is still working on the draft; approve when it waits for input",
  bundle_incomplete: "The plan links files that are not written yet",
  adrs_unpinned: "An ADR the plan proposes is not pinned at its blob; ask the agent to run plan.ts adrs-pin",
  plan_does_not_fit: "The plan does not fit the map of any workspace of the issue",
};

const DRAFT_REFUSAL: Record<string, string> = {
  unknown_draft: "Rubato does not know this draft",
};

/** Why a draft read failed: the refusal in words with its code, or what the server said. */
export function draftFailureText(e: unknown): string {
  const reason = refusalReason(e);
  if (reason === null) {
    return failureText(e);
  }
  const words = DRAFT_REFUSAL[reason];
  return words ? `${words} (${reason})` : reason;
}
