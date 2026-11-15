import { useState } from "react";
import type { IssueSummary, Me } from "../../protocol.ts";
import { ApiError, failureText } from "../../shared/api.ts";
import { IMPORT_REFUSAL, planDraftsApi, refusalReason } from "./api.ts";

/**
 * A plan written outside Rubato (ADR 0058): shown with its fit, not approvable as it stands.
 * *Import* makes it a draft you own when it fits, or starts a session of yours that rewrites it
 * into Rubato's form when it does not. Its author keeps the credit; nothing reaches their session.
 */
export function ImportPlan({ issue, me, onDone }: { issue: IssueSummary; me: Me | null; onDone: (text: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<string | null>(null);
  const branch = issue.plan?.git_ref ?? null;
  const fits = issue.fit?.fits ?? false;
  const why = !me ? "Log in to import." : !branch ? "Rubato does not know the plan's branch." : null;
  const run = async () => {
    if (!branch) {
      return;
    }
    setBusy(true);
    try {
      const r = await planDraftsApi.importPlan({ repo: issue.issue.repo, number: issue.issue.number, branch });
      const text =
        r.outcome === "imported"
          ? `Imported as your draft, revision ${r.rev}${r.original_author ? `; written by ${r.original_author}` : ""}.`
          : `A session of yours is rewriting the plan into Rubato's form on ${r.branch}.`;
      setSaid(text);
      onDone(text);
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        return;
      }
      const reason = refusalReason(e);
      const words = reason ? IMPORT_REFUSAL[reason] : undefined;
      const text = words ? `${words} (${reason})` : `Not imported: ${reason ?? failureText(e)}`;
      setSaid(text);
      onDone(text);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="import-plan" data-testid="import-plan">
      <p className="muted">Written outside Rubato: it cannot be approved as it stands. Import it to work on it here.</p>
      <button type="button" disabled={busy || why !== null} title={why ?? undefined} onClick={() => void run()}>
        Import
      </button>{" "}
      <span className="muted">{fits ? "it fits: it becomes your draft" : "it does not fit: a session of yours rewrites it"}</span>
      {said && <p role="status">{said}</p>}
    </div>
  );
}
