import type { IssueRef, IssueSummary, Me, Overview, PinRequest } from "../../protocol.ts";
import { STATE_LABEL, issueKey, relative } from "../../shared/format.ts";

interface Props {
  overview: Overview;
  me: Me | null;
  selected: IssueSummary | null;
  selectedNode: string | null;
  onSelect: (issue: IssueRef | null) => void;
  onClearNode: () => void;
  onLaunch: (issue: IssueRef) => void;
  onUnpin: (req: PinRequest) => void;
}

/** Companions the current user may start sessions in: their own, paired, connected (ADR 0004). */
export function ownConnectedCompanions(overview: Overview, me: Me | null) {
  if (!me) {
    return [];
  }
  return overview.companions.filter((c) => c.user === me.login && c.connected && !c.revoked_at);
}

const ORDER = ["blocked", "unknown", "building", "designing", "designed", "started", "in_review", "planned", "done"] as const;

export function IssuePanel({ overview, me, selected, selectedNode, onSelect, onClearNode, onLaunch, onUnpin }: Props) {
  const own = ownConnectedCompanions(overview, me);
  const issues = [...overview.issues]
    .filter((i) => !selectedNode || i.pins.some((p) => p.id === selectedNode))
    .sort((a, b) => ORDER.indexOf(a.progress.state) - ORDER.indexOf(b.progress.state) || a.issue.number - b.issue.number);

  if (selected) {
    const key = issueKey(selected.issue);
    return (
      <section data-testid="issue-detail">
        <button onClick={() => onSelect(null)}>← all issues</button>
        <h2 style={{ marginTop: 8 }}>
          {selected.url ? (
            <a href={selected.url} target="_blank" rel="noreferrer">
              {key}
            </a>
          ) : (
            key
          )}
        </h2>
        {selected.title && <p>{selected.title}</p>}
        <p>
          <span className={`badge ${selected.progress.state}`}>{STATE_LABEL[selected.progress.state]}</span> <span className="muted">last signal {relative(selected.progress.last_seen)}</span>
          {selected.progress.reason && <span className="muted"> · {selected.progress.reason}</span>}
        </p>
        {own.length > 0 ? (
          <button className="primary" data-testid="start-session" onClick={() => onLaunch(selected.issue)} disabled={selected.pins.length === 0} title={selected.pins.length === 0 ? "pin the issue to the map first" : `start in your companion ${own[0]?.hostname ?? ""}`}>
            Start session in my tmux
          </button>
        ) : (
          <p className="muted" data-testid="no-companion">
            No companion of yours is connected; start one with <code>rubato-companion run</code>.
          </p>
        )}
        <h3 style={{ marginTop: 12 }}>Pinned to</h3>
        <ul className="plain">
          {selected.pins.map((p) => (
            <li key={`${p.kind}:${p.id}`} className="issue-row">
              <span>
                <span className="muted">{p.kind}</span> {p.id}
              </span>
              <button onClick={() => onUnpin({ issue: selected.issue, target: p })}>unpin</button>
            </li>
          ))}
          {selected.pins.length === 0 && <li className="muted">not pinned</li>}
        </ul>
        {(selected.sessions ?? []).length > 0 && (
          <>
            <h3 style={{ marginTop: 12 }}>Sessions</h3>
            <table className="mini">
              <tbody>
                {(selected.sessions ?? []).map((s, i) => (
                  <tr key={s.session_id ?? i}>
                    <td>
                      <span className={`badge ${s.status}`}>{s.status}</span>
                    </td>
                    <td className="muted">{s.source}</td>
                    <td>{s.tmux?.session ?? s.cwd}</td>
                    <td className="muted">{relative(s.last_seen)}</td>
                    <td>{s.spend_usd !== undefined && s.spend_usd !== null ? `$${s.spend_usd.toFixed(2)}` : ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
        {selected.plan && (
          <>
            <h3 style={{ marginTop: 12 }}>Plan · {selected.plan.path}</h3>
            <table className="mini">
              <thead>
                <tr>
                  <th>Node</th>
                  <th>Status</th>
                  <th>Seams</th>
                </tr>
              </thead>
              <tbody>
                {selected.plan.affected_parts.map((p) => (
                  <tr key={p.node}>
                    <td>{p.node}</td>
                    <td>{p.status}</td>
                    <td className="muted">{(p.seams ?? []).join(", ")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {selected.plan.assumed && selected.plan.assumed.some((a) => a.scope === "seam") && (
              <p className="warn">⚠ {selected.plan.assumed.filter((a) => a.scope === "seam").length} seam-level question(s) wait for a human.</p>
            )}
          </>
        )}
        {selected.pull_request && (
          <p>
            PR{" "}
            <a href={selected.pull_request.url} target="_blank" rel="noreferrer">
              #{selected.pull_request.number}
            </a>{" "}
            <span className="muted">
              {selected.pull_request.merged_at ? "merged" : selected.pull_request.open ? "open" : "closed"} · checks {selected.pull_request.checks}
            </span>
          </p>
        )}
        {(selected.decisions ?? []).length > 0 && (
          <>
            <h3 style={{ marginTop: 12 }}>Decisions</h3>
            <ul className="plain">
              {(selected.decisions ?? []).map((d) => (
                <li key={d.comment_id}>
                  <a href={d.url} target="_blank" rel="noreferrer">
                    {d.title}
                  </a>{" "}
                  <span className="muted">
                    {d.author} · {relative(d.created_at)}
                  </span>
                  {d.chosen && <div className="muted">chosen: {d.chosen}</div>}
                </li>
              ))}
            </ul>
          </>
        )}
        {selected.spend_usd !== undefined && selected.spend_usd !== null && <p className="muted">spend ${selected.spend_usd.toFixed(2)}</p>}
      </section>
    );
  }

  return (
    <section data-testid="issue-list">
      {selectedNode && (
        <p>
          <span className="muted">on</span> <strong>{selectedNode}</strong> <button onClick={onClearNode}>show all</button>
        </p>
      )}
      <ul className="plain">
        {issues.map((i) => (
          <li key={issueKey(i.issue)} className="issue-row" onClick={() => onSelect(i.issue)} data-testid={`issue-${i.issue.number}`}>
            <span>
              <strong>{issueKey(i.issue)}</strong> <span className="muted">{i.title ?? ""}</span>
              <div className="muted">{i.pins.map((p) => p.id).join(", ") || "not pinned"}</div>
            </span>
            <span className={`badge ${i.progress.state}`}>{STATE_LABEL[i.progress.state]}</span>
          </li>
        ))}
        {issues.length === 0 && <li className="muted">No issues yet. Pin one to a node or seam.</li>}
      </ul>
      {(overview.unpinned_sessions ?? []).length > 0 && (
        <p className="muted">
          {(overview.unpinned_sessions ?? []).length} session{(overview.unpinned_sessions ?? []).length === 1 ? "" : "s"} on branches outside the convention (unpinned).
        </p>
      )}
    </section>
  );
}
