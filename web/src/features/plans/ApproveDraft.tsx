import { useEffect, useId, useState } from "react";
import type { DraftApproveResponse, DraftSummary, IssueSummary, Me, SessionLogEntry } from "../../protocol.ts";
import { ApiError, failureText } from "../../shared/api.ts";
import { relative } from "../../shared/format.ts";
import { chatApi } from "../chat/index.ts";
import { DRAFT_APPROVE_REFUSAL, planDraftsApi, refusalReason } from "./api.ts";

const sameLogin = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** The text of a `Summary:` reply without its label: what the approver edits. */
export function summaryText(entry: SessionLogEntry): string {
  return entry.text.replace(/^\s*Summary:\s*/, "").trim();
}

/** The newest entry of `kind` across the logs, by time. */
export function newestOfKind(entries: SessionLogEntry[], kind: SessionLogEntry["kind"]): SessionLogEntry | null {
  let best: SessionLogEntry | null = null;
  for (const e of entries) {
    if (e.kind === kind && (best === null || e.at > best.at)) {
      best = e;
    }
  }
  return best;
}

/** The launches whose log speaks for this draft: the owner's launched sessions on its branch. */
export function draftLaunches(issue: IssueSummary, d: DraftSummary): string[] {
  const ids = (issue.sessions ?? []).filter((s) => s.launch_id && sameLogin(s.user, d.owner) && s.branch === d.branch).map((s) => s.launch_id as string);
  return [...new Set(ids)];
}

interface Discussion {
  review: SessionLogEntry | null;
  summary: SessionLogEntry | null;
}

/** The agent's newest `Review:` and `Summary:` replies for the draft (ADR 0056), read once. */
function useDiscussion(launches: string[]): Discussion | null {
  const key = launches.join(",");
  const [found, setFound] = useState<{ key: string; d: Discussion } | null>(null);
  useEffect(() => {
    let current = true;
    const ids = key ? key.split(",") : [];
    Promise.all(ids.map((id) => chatApi.log(id).then((l: { entries: SessionLogEntry[] }) => l.entries, () => [] as SessionLogEntry[]))).then((logs) => {
      if (current) {
        const all = logs.flat();
        setFound({ key, d: { review: newestOfKind(all, "review"), summary: newestOfKind(all, "summary") } });
      }
    });
    return () => {
      current = false;
    };
  }, [key]);
  return found?.key === key ? found.d : null;
}

interface Props {
  issue: IssueSummary;
  d: DraftSummary;
  me: Me | null;
  /** The newest settled revision the person is looking at. */
  rev: number;
}

/**
 * Approve one revision of a draft in Rubato (ADR 0054), as yourself: the plan, its design note
 * and its ADRs as they are in that revision. The summary starts as the agent's newest
 * `Summary:` reply and is yours to edit; it is kept with the approval and posted only in the
 * link comment after handoff (ADR 0055). Nothing is posted on GitHub here.
 */
export function ApproveDraft({ issue, d, me, rev }: Props) {
  const heading = useId();
  const summaryId = useId();
  const discussion = useDiscussion(draftLaunches(issue, d));
  const [summary, setSummary] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<DraftApproveResponse | null>(null);
  const [said, setSaid] = useState<string | null>(null);

  // Pre-filled once, when the discussion is read; never over what the person typed.
  useEffect(() => {
    if (discussion && summary === null) {
      setSummary(discussion.summary ? summaryText(discussion.summary) : "");
    }
  }, [discussion, summary]);

  const record = issue.approval?.record;
  const approvedHere = record?.source === "rubato" && record.sha === d.plan_sha && issue.approval?.version === "current";
  const mine = me !== null && sameLogin(me.login, d.owner);
  const missing = d.missing ?? [];
  const why = !me
    ? "Log in to approve."
    : d.parse_error
      ? "The plan does not parse; it cannot be approved."
      : missing.length > 0
        ? "The plan links files that are not written yet."
        : null;

  if (approvedHere && record) {
    return (
      <p className="approved" data-testid="draft-approved">
        <span className="badge approved">approved</span> revision {rev} by {record.author}
        {sameLogin(record.author, d.owner) ? " (self)" : ""},{" "}
        <time dateTime={record.created_at} title={new Date(record.created_at).toLocaleString()}>
          {relative(record.created_at)}
        </time>
        . It reaches the branch at handoff.
      </p>
    );
  }

  const approve = async () => {
    setBusy(true);
    setSaid(null);
    try {
      const r = await planDraftsApi.approve({
        repo: issue.issue.repo,
        number: issue.issue.number,
        branch: d.branch,
        plan_sha: d.plan_sha,
        rev,
        summary: summary ?? "",
      });
      setDone(r);
      setSaid(r.created ? `Approved revision ${rev} as you${r.self_approved ? " (self)" : ""}.` : `Revision ${rev} was already approved by ${r.approved_by}.`);
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        return;
      }
      const reason = refusalReason(e);
      const words = reason ? DRAFT_APPROVE_REFUSAL[reason] : undefined;
      setSaid(words ? `${words} (${reason})` : `Not approved: ${reason ?? failureText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="approve-draft" aria-labelledby={heading} data-testid="approve-draft">
      <h4 id={heading}>Approve revision {rev}</h4>
      {discussion?.review && (
        <details className="draft-review">
          <summary>
            The agent's review,{" "}
            <time dateTime={discussion.review.at} title={new Date(discussion.review.at).toLocaleString()}>
              {relative(discussion.review.at)}
            </time>
            : {discussion.review.text.split("\n")[0]}
          </summary>
          <pre className="draft-text">{discussion.review.text}</pre>
        </details>
      )}
      {discussion && !discussion.review && <p className="muted">The agent has not replied with a Review: yet.</p>}
      <label htmlFor={summaryId}>Summary of the discussion</label>
      <textarea
        id={summaryId}
        rows={4}
        value={summary ?? ""}
        disabled={summary === null || done !== null}
        placeholder={summary === null ? "Reading the agent's summary…" : "What was discussed and decided; posted with the link comment"}
        onChange={(e) => setSummary(e.target.value)}
      />
      <p className="muted">
        {discussion?.summary ? "From the agent's newest Summary: reply; edit it as you like. " : ""}
        Kept with the approval and posted only in the link comment after handoff; never committed.
      </p>
      {done === null && (
        <p>
          <button type="button" className="primary" disabled={busy || why !== null || summary === null} title={why ?? undefined} onClick={() => void approve()}>
            Approve
          </button>{" "}
          <span className="muted">
            plan <code>{d.plan_sha.slice(0, 10)}</code> · as you{mine ? ", who own this draft: recorded as self-approval" : ""} · nothing is posted on GitHub
          </span>
        </p>
      )}
      {why && done === null && (
        <p className="muted" data-testid="approve-why">
          {why}
        </p>
      )}
      {said && <p role="status">{said}</p>}
    </section>
  );
}
