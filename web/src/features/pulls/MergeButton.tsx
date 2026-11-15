import { useState } from "react";
import type { MergeMethod, PullDetail } from "../../protocol.ts";
import { PLAN_WHY, WHY_NOT, pullsApi, refusalOf } from "./api.ts";

const METHOD_TEXT: Record<MergeMethod, string> = { merge: "Merge commit", squash: "Squash and merge", rebase: "Rebase and merge" };

/** Merge as yourself (ADR 0033): only the methods the repo allows, one click, at the head shown. */
export function MergeButton({ detail, onMerged }: { detail: PullDetail; onMerged?: (() => void) | undefined }) {
  const offer = detail.merge;
  const methods = offer.methods ?? [];
  const [method, setMethod] = useState<MergeMethod | undefined>(offer.default_method ?? methods[0]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const why = offer.why_not ?? null;
  const gate = offer.plan_gate.kind === "not_approved" ? (PLAN_WHY[offer.plan_gate.why] ?? offer.plan_gate.why) : null;

  async function merge() {
    if (!method) {
      return;
    }
    setBusy(true);
    setResult(null);
    try {
      const r = await pullsApi.merge({ repo: detail.repo, number: detail.number, head_sha: detail.head_sha, method });
      setResult(r.state === "merged" ? `Merged by ${r.login}.` : "Sent to GitHub; not confirmed yet. Rubato checks again within a few minutes.");
      onMerged?.();
    } catch (e) {
      setResult(refusalOf(e).text);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="merge" data-testid="merge">
      {methods.length > 0 && (
        <label>
          Method{" "}
          <select value={method} onChange={(e) => setMethod(e.target.value as MergeMethod)} disabled={!offer.can_merge || busy}>
            {methods.map((m) => (
              <option key={m} value={m}>
                {METHOD_TEXT[m]}
              </option>
            ))}
          </select>
        </label>
      )}{" "}
      <button type="button" className="primary" disabled={!offer.can_merge || busy || !method} onClick={() => void merge()}>
        {busy ? "Merging…" : "Merge"}
      </button>
      {why && (
        <p className="muted" data-testid="merge-why-not">
          {WHY_NOT[why] ?? why}
          {why === "plan_not_approved" && gate ? ` (${gate})` : ""}
        </p>
      )}
      {(offer.stacked_above ?? 0) > 0 && (
        <p className="warn" data-testid="merge-stacked">
          {offer.stacked_above} open pull request{offer.stacked_above === 1 ? " is" : "s are"} based on this branch; a squash makes {offer.stacked_above === 1 ? "it" : "them"} conflict. A merge commit keeps {offer.stacked_above === 1 ? "it" : "them"} mergeable.
        </p>
      )}
      <p role="status" data-testid="merge-result">
        {result}
      </p>
    </div>
  );
}
