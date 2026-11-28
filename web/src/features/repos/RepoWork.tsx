import { PinMenu } from "../pins/index.ts";
import { PullList } from "../pulls/index.ts";
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
  // An unpinned issue with a plan is in both lists: count it once, as tracked.
  const seen = new Set(tracked.map((r) => `${r.issue.repo.toLowerCase()}#${r.issue.number}`));
  const rest = backlog.filter((r) => !seen.has(`${r.issue.repo.toLowerCase()}#${r.issue.number}`));
  return [...tracked, ...rest].sort((a, b) => b.issue.number - a.issue.number);
}

/** A repo's open pull requests and open issues (issue #82), in the repo's own view (issue #357). */
export function RepoWork({ overview, repo, onOpenIssue, onOpenPull, onPin }: { overview: Overview; repo: string; onOpenIssue?: ((i: IssueRef) => void) | undefined; onOpenPull: (number: number) => void; onPin?: ((i: IssueRef, t: MapRef) => void) | undefined }) {
  const listing = (overview.pulls ?? []).find((p) => p.repo.toLowerCase() === repo.toLowerCase());
  const pulls = listing?.pulls ?? [];
  const issues = openIssuesOf(overview, repo);
  return (
    <section className="repo-work" data-testid={`repo-work-${repo}`}>
      <h3 data-testid="repo-work-pulls">
        {pulls.length} open pull request{pulls.length === 1 ? "" : "s"}
      </h3>
      {listing?.error && <p className="warn">The last listing failed: {listing.error}. These are from the one before.</p>}
      <PullList pulls={pulls} truncated={listing?.truncated ?? false} onOpen={(p) => onOpenPull(p.number)} />
      <h3 data-testid="repo-work-issues">
        {issues.length} open issue{issues.length === 1 ? "" : "s"}
      </h3>
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
    </section>
  );
}
