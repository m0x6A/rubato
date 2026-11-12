import { useState } from "react";
import { PinMenu } from "../pins/index.ts";
import { QueueMenu } from "../queue/index.ts";
import type { Backlog as BacklogData, BacklogRepo, IssueRef, MapRef, OpenIssue, PairingInfo, QueuedIssue, SystemMap } from "../../protocol.ts";
import { setIssueDrag } from "../../shared/dnd.ts";
import { issueKey, relative } from "../../shared/format.ts";

interface Props {
  backlog: BacklogData | undefined;
  map: SystemMap;
  onPin: (issue: IssueRef, target: MapRef) => void;
  /** The caller's own companions, for "Queue on…". */
  lanes?: PairingInfo[] | undefined;
  queue?: QueuedIssue[] | undefined;
  onQueue?: ((issue: IssueRef, companionId: string) => void) | undefined;
}

/** "5 min ago" under an hour, as the design words it; the shared short form beyond that. */
function ago(iso: string): string {
  const mins = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60_000));
  if (mins < 1) {
    return "just now";
  }
  return mins < 60 ? `${mins} min ago` : relative(iso);
}

const failed = (r: BacklogRepo) => Boolean(r.error);
const listed = (r: BacklogRepo) => Boolean(r.listed_at) && !failed(r);

/** How fresh one repo's listing is: the trust check for the list above it. */
function RepoStatus({ repo }: { repo: BacklogRepo }) {
  let text: string;
  if (repo.error) {
    text = `could not be listed: ${repo.error} · ${repo.listed_at ? `last listed ${ago(repo.listed_at)}` : "never listed"}`;
  } else if (repo.listed_at) {
    text = `listed ${ago(repo.listed_at)}`;
  } else {
    text = "not listed yet";
  }
  return (
    <li className={repo.error ? "warn" : "muted"} data-testid={`backlog-repo-${repo.repo}`}>
      <strong>{repo.repo}</strong> {text}
      {repo.truncated ? " · first 500 shown" : ""}
    </li>
  );
}

/** A suggested place (ADR 0026), confirmed with one click. Nothing is pinned until then. */
function Suggestion({ issue, onPin }: { issue: OpenIssue; onPin: (issue: IssueRef, target: MapRef) => void }) {
  const s = issue.suggestion;
  if (!s) {
    return null;
  }
  const where = `${s.target.kind === "seam" ? "seam" : "part"} ${s.target.id}`;
  const from = issue.map_line ? "its Map: line, from someone outside the team" : (s.why ?? []).join("; ");
  return (
    <span className="suggestion" data-testid={`suggestion-${issue.issue.number}`}>
      Suggested: <strong>{where}</strong>
      {from && <span className="muted"> ({from})</span>}{" "}
      <button type="button" onClick={() => onPin(issue.issue, s.target)} aria-label={`Pin ${issueKey(issue.issue)} to ${where}`}>
        Pin here
      </button>
    </span>
  );
}

function matches(i: OpenIssue, needle: string): boolean {
  return [issueKey(i.issue), i.title, ...(i.labels ?? [])].some((s) => s.toLowerCase().includes(needle));
}

/**
 * "Not pinned": the open issues of the map's repos that Rubato does not track yet (design S2).
 * A failed or missing listing is said per repo; an empty list is claimed only for listed repos.
 */
export function Backlog({ backlog, map, onPin, lanes = [], queue = [], onQueue }: Props) {
  const [filter, setFilter] = useState("");
  const all = backlog?.issues ?? [];
  const repos = backlog?.repos ?? [];
  const needle = filter.trim().toLowerCase();
  const shown = needle ? all.filter((i) => matches(i, needle)) : all;
  const listedRepos = repos.filter(listed);

  return (
    <section className="backlog" data-testid="backlog" aria-labelledby="backlog-heading">
      <h3 id="backlog-heading">Not pinned ({all.length})</h3>
      {repos.length === 0 ? (
        <p className="muted">The map names no repository, so there are no open issues to list. Give a part a repo in the system map.</p>
      ) : (
        <>
          {all.length > 0 && <p className="muted">Drag an issue onto a part or a seam, or use Pin to…</p>}
          {all.length > 0 && <input type="search" aria-label="Filter not-pinned issues" placeholder="Filter by key, title or label" value={filter} onChange={(e) => setFilter(e.target.value)} />}
          <ul className="plain">
            {shown.map((i) => (
              <li key={issueKey(i.issue)} className="issue-row backlog-row" data-testid={`backlog-row-${i.issue.number}`} draggable onDragStart={(e) => setIssueDrag(e, i.issue)} title="Drag onto a part or a seam of the map to pin">
                <span className="issue-main">
                  {/* Not draggable on its own: a drag that starts on the link drags the row, not a URL. */}
                  <a href={i.url} target="_blank" rel="noreferrer" draggable={false}>
                    {issueKey(i.issue)}
                  </a>{" "}
                  <span>{i.title}</span>
                  {(i.labels ?? []).length > 0 && (
                    <span className="labels">
                      {(i.labels ?? []).map((l) => (
                        <span key={l} className="badge">
                          {l}
                        </span>
                      ))}
                    </span>
                  )}
                  <Suggestion issue={i} onPin={onPin} />
                </span>
                <span className="row-end">
                  <PinMenu map={map} issue={i.issue} onPin={onPin} />
                  {onQueue && <QueueMenu issue={i.issue} lanes={lanes} queue={queue} onQueue={onQueue} />}
                </span>
              </li>
            ))}
          </ul>
          {all.length > 0 && shown.length === 0 && <p className="muted">No not-pinned issue matches “{filter.trim()}”.</p>}
          {all.length === 0 && listedRepos.length > 0 && <p className="muted">No open issues in {listedRepos.map((r) => r.repo).join(", ")} waiting to be pinned.</p>}
          <ul className="plain repo-status" aria-label="How fresh each repository's listing is">
            {repos.map((r) => (
              <RepoStatus key={r.repo} repo={r} />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
