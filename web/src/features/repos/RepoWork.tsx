import { useState } from "react";
import { PinMenu } from "../pins/index.ts";
import { PullList, PullView } from "../pulls/index.ts";
import type { IssueRef, MapRef, Overview } from "../../protocol.ts";
import { isFinished } from "../../shared/finished.ts";
import { relative } from "../../shared/format.ts";

type Row = { issue: IssueRef; title: string; labels: string[]; updated_at: string | null; url: string | null; tracked: boolean };

/** One repo's open issues as Rubato stores them (ADR 0026: no body), backlog and tracked. */
export function openIssuesOf(overview: Overview, repo: string): Row[] {
  const same = (r: string) => r.toLowerCase() === repo.toLowerCase();
  const backlog: Row[] = (overview.backlog?.issues ?? [])
    .filter((i) => same(i.issue.repo))
    .map((i) => ({ issue: i.issue, title: i.title, labels: i.labels ?? [], updated_at: i.updated_at, url: i.url, tracked: false }));
  const tracked: Row[] = overview.issues
    .filter((i) => same(i.issue.repo) && !isFinished(i))
    .map((i) => ({ issue: i.issue, title: i.title ?? `#${i.issue.number}`, labels: [], updated_at: null, url: i.url ?? null, tracked: true }));
  return [...tracked, ...backlog].sort((a, b) => b.issue.number - a.issue.number);
}

/** A repo's open pull requests and open issues, folded under its rack unit (issue #82). */
export function RepoWork({ overview, repo, onOpenIssue, onPin }: { overview: Overview; repo: string; onOpenIssue?: ((i: IssueRef) => void) | undefined; onPin?: ((i: IssueRef, t: MapRef) => void) | undefined }) {
  const [open, setOpen] = useState<number | null>(null);
  const listing = (overview.pulls ?? []).find((p) => p.repo.toLowerCase() === repo.toLowerCase());
  const pulls = listing?.pulls ?? [];
  const issues = openIssuesOf(overview, repo);
  return (
    <details className="repo-work" data-testid={`repo-work-${repo}`}>
      <summary>
        {pulls.length} open pull request{pulls.length === 1 ? "" : "s"} · {issues.length} open issue{issues.length === 1 ? "" : "s"}
      </summary>
      <h4>Pull requests</h4>
      {listing?.error && <p className="warn">The last listing failed: {listing.error}. These are from the one before.</p>}
      <PullList pulls={pulls} truncated={listing?.truncated ?? false} onOpen={(p) => setOpen(open === p.number ? null : p.number)} />
      {open !== null && <PullView repo={repo} number={open} refreshKey={overview.generated_at} onOpenIssue={onOpenIssue} />}
      <h4>Issues</h4>
      {issues.length === 0 && <p className="muted">No open issues listed.</p>}
      <ul className="plain" data-testid="repo-issues">
        {issues.map((i) => (
          <li key={i.issue.number} data-testid={`repo-issue-${i.issue.number}`}>
            {i.tracked && onOpenIssue ? (
              <button type="button" className="link" onClick={() => onOpenIssue(i.issue)}>
                #{i.issue.number} {i.title}
              </button>
            ) : (
              <span>
                #{i.issue.number} {i.title}
              </span>
            )}{" "}
            {i.labels.map((l) => (
              <span key={l} className="badge in">
                {l}
              </span>
            ))}{" "}
            {!i.tracked && onPin && <PinMenu map={overview.map.map} issue={i.issue} onPin={onPin} />}
            <div className="muted">
              {i.tracked ? "tracked" : "not pinned"}
              {i.updated_at ? ` · updated ${relative(i.updated_at)}` : ""}
              {i.url && (
                <>
                  {" · "}
                  <a href={i.url} target="_blank" rel="noreferrer">
                    on GitHub
                  </a>
                </>
              )}
            </div>
          </li>
        ))}
      </ul>
    </details>
  );
}
