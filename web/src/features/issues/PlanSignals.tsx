import type { IssueSummary } from "../../protocol.ts";
import { approvalClass, approvalLabel, relative, reviewLabel } from "../../shared/format.ts";

/** Review and approval of an issue's plan, read from `Review:` and `Approved:` comments by team
 * members (ADR 0027). Nothing here is written into the plan file. */
export function PlanSignals({ issue }: { issue: IssueSummary }) {
  const { review, approval } = issue;
  return (
    <div data-testid="plan-signals">
      <h3 style={{ marginTop: 12 }}>Review and approval</h3>
      <ul className="plain">
        <li data-testid="plan-review">
          {review ? (
            <>
              <a href={review.url} target="_blank" rel="noreferrer">
                {reviewLabel(review)}
              </a>{" "}
              <span className="muted">
                <code>{review.author}</code> · {relative(review.created_at)}
              </span>
              {review.left_open && <div className="muted">left open: {review.left_open}</div>}
            </>
          ) : (
            <span className="muted">not reviewed</span>
          )}
        </li>
        <li data-testid="plan-approval">
          {approval ? (
            <>
              <span className={`badge ${approvalClass(approval)}`}>{approvalLabel(approval)}</span>{" "}
              <a href={approval.record.url} target="_blank" rel="noreferrer">
                by <code>{approval.record.author}</code>
              </a>{" "}
              <span className="muted">
                {approval.version === "current"
                  ? "current version"
                  : approval.version === "earlier"
                    ? "the plan changed since"
                    : approval.version === "does_not_fit"
                      ? "the plan does not fit the map (ADR 0029)"
                      : ""}
                {approval.record.branch ? ` · on ${approval.record.branch}` : ""} · {relative(approval.record.created_at)}
              </span>
            </>
          ) : (
            <span className="muted">
              not approved: a team member comments <code>Approved: plan &lt;sha&gt;</code>
            </span>
          )}
        </li>
      </ul>
      <p className="muted">Read from issue comments by team members, at least every 5 minutes.</p>
    </div>
  );
}

/** The compact badges on an issue row: reviewed, approved or approval stale; and, with several
 * workspaces (issue #89), where else it is pinned and where its plan counts when not here. */
export function SignalBadges({ issue, here }: { issue: IssueSummary; here?: string | undefined }) {
  const fitsIn = issue.fits_in ?? [];
  const countsElsewhere = fitsIn.length > 0 && here !== undefined && !fitsIn.includes(here);
  return (
    <>
      {(issue.pinned_in ?? []).length > 0 && (
        <span className="badge elsewhere" title="Pinned in another workspace">
          pinned in {(issue.pinned_in ?? []).join(", ")}
        </span>
      )}
      {countsElsewhere && (
        <span className="badge elsewhere" title="The plan fits that workspace's map, so its approval counts there">
          counts in {fitsIn.join(", ")}
        </span>
      )}
      {issue.review && (
        <span className="badge reviewed" title={reviewLabel(issue.review)}>
          reviewed
        </span>
      )}
      {issue.approval && (
        <span className={`badge ${approvalClass(issue.approval)}`} title={approvalLabel(issue.approval)}>
          {issue.approval.version === "current"
            ? "approved"
            : issue.approval.version === "earlier"
              ? "approval stale"
              : issue.approval.version === "does_not_fit"
                ? "approved, doesn't fit"
                : "approved, unchecked"}
        </span>
      )}
    </>
  );
}
