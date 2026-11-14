import type { ApproveRequest, ApproveResponse, DraftRevision, DraftView } from "../../protocol.ts";
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
};

/** A plan draft and its revisions (ADR 0053): team routes, never `scoped()`. */
export const planDraftsApi = {
  draft: (draftId: string) => request<DraftView>("GET", `/api/drafts/${encodeURIComponent(draftId)}`),
  revision: (draftId: string, rev: number) => request<DraftRevision>("GET", `/api/drafts/${encodeURIComponent(draftId)}/revisions/${rev}`),
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
