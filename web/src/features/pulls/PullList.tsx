import type { ChecksStatus, OpenPull } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";

/** The badge class and words for a checks status, from existing badge colours. */
export function checksBadge(c: ChecksStatus | null | undefined): { cls: string; text: string } {
  switch (c) {
    case "green":
      return { cls: "badge done", text: "checks green" };
    case "red":
      return { cls: "badge blocked", text: "checks red" };
    case "pending":
      return { cls: "badge working", text: "checks running" };
    case "none":
      return { cls: "badge unknown", text: "no checks" };
    default:
      return { cls: "badge unknown", text: "checks not read yet" };
  }
}

/** One repo's open pull requests (ADR 0034): a row each; choosing one opens its view. */
export function PullList({ pulls, onOpen, truncated = false }: { pulls: OpenPull[]; onOpen: (p: OpenPull) => void; truncated?: boolean }) {
  if (pulls.length === 0) {
    return <p className="muted">No open pull requests from this repo's branches.</p>;
  }
  return (
    <>
      <ul className="plain" data-testid="pull-list">
        {pulls.map((p) => {
          const checks = checksBadge(p.checks);
          return (
            <li key={p.number} data-testid={`pull-row-${p.number}`}>
              <button type="button" className="link" onClick={() => onOpen(p)}>
                #{p.number} {p.title}
              </button>{" "}
              {p.draft && <span className="badge draft-badge">draft</span>} <span className={checks.cls}>{checks.text}</span>
              <div className="muted">
                {p.author} · <code>{p.branch}</code> → <code>{p.base}</code> · {relative(p.updated_at)}
                {p.issue ? ` · issue #${p.issue.number}` : ""}
              </div>
            </li>
          );
        })}
      </ul>
      {truncated && <p className="muted">Only the first 100 are listed.</p>}
    </>
  );
}
