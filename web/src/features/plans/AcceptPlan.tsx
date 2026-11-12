import { useState } from "react";
import type { IssueSummary, Me } from "../../protocol.ts";
import { failureText } from "../../shared/api.ts";
import { REFUSAL_TEXT, plansApi, refusalReason } from "./api.ts";

const CANNOT_ACT: Record<string, string> = {
  oauth_only: "This Rubato is not a GitHub App, so it cannot comment as you.",
  no_token_key: "This Rubato keeps no tokens (RUBATO_TOKEN_KEY is not set).",
  tokens_do_not_expire: "The GitHub App issues tokens that do not expire; Rubato keeps none.",
  log_in_again: "Log in again so Rubato can comment as you.",
};

/** *Accept plan*: posts `Approved: plan <sha> on <branch>` as you (ADR 0030). One click, no
 * dialog: it is idempotent for you and this plan version, and a changed plan needs a new one. */
export function AcceptPlan({ issue, me, onDone }: { issue: IssueSummary; me: Me | null; onDone: (text: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<string | null>(null);
  const sha = issue.plan_sha;
  const counted = issue.approval?.version === "current";
  const fits = issue.fit?.fits ?? false;
  const why = !sha
    ? "There is no plan file to accept."
    : !fits
      ? "The plan does not fit the map; fix the reasons above first."
      : me && !me.can_act_on_github
        ? (CANNOT_ACT[me.cannot_act_reason ?? ""] ?? "Rubato cannot comment as you.")
        : null;
  if (counted) {
    return null;
  }
  const accept = async () => {
    if (!sha) {
      return;
    }
    setBusy(true);
    try {
      const r = await plansApi.approve({ issue: issue.issue, sha });
      const text = r.state === "posted" ? `Accepted: posted "Approved: plan ${sha.slice(0, 10)}" as you.` : "Approval posted, waiting for GitHub to confirm it.";
      setSaid(text);
      onDone(text);
    } catch (e) {
      const reason = refusalReason(e);
      const text = (reason && REFUSAL_TEXT[reason]) ?? `The plan was not accepted: ${failureText(e)}`;
      setSaid(text);
      onDone(text);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="accept-plan" data-testid="accept-plan">
      <button type="button" className="primary" disabled={busy || why !== null} title={why ?? undefined} onClick={() => void accept()}>
        Accept plan
      </button>{" "}
      {sha && (
        <span className="muted">
          plan <code>{sha.slice(0, 10)}</code>
          {issue.plan?.git_ref ? ` on ${issue.plan.git_ref}` : ""} · posted as you
        </span>
      )}
      {why && <div className="muted" data-testid="accept-why">{why}</div>}
      {said && <div role="status">{said}</div>}
    </div>
  );
}
