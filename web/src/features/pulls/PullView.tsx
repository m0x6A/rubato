import { useEffect, useState } from "react";
import type { IssueRef, PullDetail } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";
import { checksBadge } from "./PullList.tsx";
import { MergeButton } from "./MergeButton.tsx";
import { pullsApi, refusalOf } from "./api.ts";

/** A pull request's view (design 82 §5): read when opened and again when `refreshKey` changes
 * (a new overview after a merge or a poll). Title and body are text, never markup. */
export function PullView({ repo, number, refreshKey, onOpenIssue }: { repo: string; number: number; refreshKey?: string | undefined; onOpenIssue?: ((i: IssueRef) => void) | undefined }) {
  const [detail, setDetail] = useState<PullDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    let live = true;
    setError(null);
    pullsApi.view(repo, number).then(
      (d) => live && setDetail(d),
      (e: unknown) => live && setError(refusalOf(e).text),
    );
    return () => {
      live = false;
    };
  }, [repo, number, refreshKey, nonce]);

  if (error && !detail) {
    return (
      <section className="pull-view" data-testid="pull-view">
        <p className="warn" role="alert">
          Could not read {repo}#{number}: {error}
        </p>
        <button type="button" onClick={() => setNonce((n) => n + 1)}>
          Try again
        </button>
      </section>
    );
  }
  if (!detail) {
    return (
      <section className="pull-view" data-testid="pull-view" aria-busy="true">
        <p className="muted">Reading {repo}#{number} from GitHub…</p>
      </section>
    );
  }
  const d = detail;
  const checks = checksBadge(d.checks);
  const files = d.files;
  return (
    <section className="pull-view" data-testid="pull-view">
      <h3>
        <a href={d.url} target="_blank" rel="noreferrer">
          #{d.number}
        </a>{" "}
        {d.title}
      </h3>
      <p className="muted">
        <span className={d.state === "merged" ? "badge done" : d.state === "open" ? "badge in" : "badge ended"}>{d.state}</span> {d.draft && <span className="badge draft-badge">draft</span>} {d.author} · <code>{d.branch}</code> → <code>{d.base}</code>
        {d.merged_at ? ` · merged ${relative(d.merged_at)}` : ""}
      </p>
      {d.issue && (
        <p>
          Issue{" "}
          {onOpenIssue ? (
            <button type="button" className="link" onClick={() => onOpenIssue(d.issue!)}>
              #{d.issue.number}
            </button>
          ) : (
            `#${d.issue.number}`
          )}
        </p>
      )}
      {d.body && <p className="pull-body" style={{ whiteSpace: "pre-wrap" }} data-testid="pull-body">{d.body}</p>}
      <h4>
        Checks <span className={checks.cls}>{checks.text}</span>
      </h4>
      {(d.check_runs ?? []).length > 0 && (
        <ul className="plain">
          {(d.check_runs ?? []).map((r, i) => (
            <li key={`${r.name}-${i}`}>
              <a href={r.url} target="_blank" rel="noreferrer">
                {r.name}
              </a>{" "}
              <span className="muted">{r.conclusion ?? r.status}</span>
            </li>
          ))}
        </ul>
      )}
      <h4>Reviews</h4>
      {(d.reviews ?? []).length === 0 && <p className="muted">No reviews from the team.</p>}
      <ul className="plain">
        {(d.reviews ?? []).map((r) => (
          <li key={r.author}>
            {r.author} <span className="muted">{r.state.toLowerCase().replace("_", " ")} · {relative(r.submitted_at)}</span>
          </li>
        ))}
      </ul>
      {(d.outside_reviews ?? 0) > 0 && <p className="muted">{d.outside_reviews} review{d.outside_reviews === 1 ? "" : "s"} from outside the team, not shown.</p>}
      <h4>Files</h4>
      <p className="muted" data-testid="pull-files">
        {files.count}
        {files.truncated ? "+" : ""} file{files.count === 1 ? "" : "s"} · +{files.additions} −{files.deletions} ·{" "}
        <a href={`${d.url}/files`} target="_blank" rel="noreferrer">
          View diff on GitHub
        </a>
      </p>
      {(files.top ?? []).length > 0 && (
        <ul className="plain">
          {(files.top ?? []).map((f) => (
            <li key={f.path}>
              <code>{f.path}</code> <span className="muted">+{f.additions} −{f.deletions}</span>
            </li>
          ))}
        </ul>
      )}
      {d.mergeable === null || d.mergeable === undefined ? <p className="muted">GitHub is still checking whether it can merge.</p> : null}
      <MergeButton detail={d} onMerged={() => setNonce((n) => n + 1)} />
    </section>
  );
}
