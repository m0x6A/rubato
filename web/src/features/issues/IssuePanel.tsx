import { useEffect, useRef } from "react";
import { PinMenu } from "../pins/index.ts";
import type { IssueRef, IssueSummary, MapRef, Me, Overview, PinRequest } from "../../protocol.ts";
import { setIssueDrag } from "../../shared/dnd.ts";
import { STATE_LABEL, issueKey, relative } from "../../shared/format.ts";
import { Backlog } from "./Backlog.tsx";

interface Props {
  overview: Overview;
  me: Me | null;
  selected: IssueSummary | null;
  /** The part or seam selected on the map; the list shows the issues pinned to exactly it. */
  selection: MapRef | null;
  onSelect: (issue: IssueRef | null) => void;
  onClearSelection: () => void;
  onLaunch: (issue: IssueRef) => void;
  /** Adds a pin; an issue may have several. The keyboard twin of dropping a row on the map. */
  onPin: (issue: IssueRef, target: MapRef) => void;
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

export function IssuePanel({ overview, me, selected, selection, onSelect, onClearSelection, onLaunch, onPin, onUnpin }: Props) {
  const own = ownConnectedCompanions(overview, me);
  // The list gives way to the detail, and a row that was just pinned leaves "Not pinned" with the
  // button that had focus: put focus on the detail's heading when an issue opens.
  const heading = useRef<HTMLHeadingElement>(null);
  const openKey = selected ? issueKey(selected.issue) : null;
  useEffect(() => {
    if (openKey) {
      heading.current?.focus();
    }
  }, [openKey]);
  const issues = [...overview.issues]
    .filter((i) => !selection || i.pins.some((p) => p.kind === selection.kind && p.id === selection.id))
    .sort((a, b) => ORDER.indexOf(a.progress.state) - ORDER.indexOf(b.progress.state) || a.issue.number - b.issue.number);

  if (selected) {
    const key = issueKey(selected.issue);
    return (
      <section data-testid="issue-detail">
        <button onClick={() => onSelect(null)}>← all issues</button>
        <h2 style={{ marginTop: 8 }} ref={heading} tabIndex={-1}>
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
        <p style={{ marginTop: 8 }}>
          <PinMenu map={overview.map.map} issue={selected.issue} pinned={selected.pins} onPin={onPin} />
        </p>
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
      {selection && (
        <p data-testid="map-filter">
          <span className="muted">pinned to {selection.kind === "seam" ? "seam" : "part"}</span> <strong>{selection.id}</strong> <button onClick={onClearSelection}>show all</button>
        </p>
      )}
      <ul className="plain">
        {issues.map((i) => (
          <li key={issueKey(i.issue)} className="issue-row" onClick={() => onSelect(i.issue)} data-testid={`issue-${i.issue.number}`} draggable onDragStart={(e) => setIssueDrag(e, i.issue)} title="Drag onto a part or a seam of the map to add a pin">
            {/* `issue-row-N` names the whole row for the e2e; `issue-N` is the older id on the same row. */}
            <div className="row" data-testid={`issue-row-${i.issue.number}`}>
              <span className="issue-main">
                {/* A real button, so the row opens from the keyboard; its click reaches the row's handler. */}
                <button type="button" className="link">
                  {issueKey(i.issue)}
                </button>{" "}
                <span className="muted">{i.title ?? ""}</span>
                <div className="muted">{i.pins.map((p) => p.id).join(", ") || "not pinned"}</div>
              </span>
              <span className="row-end">
                <span className={`badge ${i.progress.state}`}>{STATE_LABEL[i.progress.state]}</span>
                <PinMenu map={overview.map.map} issue={i.issue} pinned={i.pins} onPin={onPin} />
              </span>
            </div>
          </li>
        ))}
        {issues.length === 0 && (
          <li className="muted" data-testid="no-tracked-issues">
            {selection
              ? `No issue is pinned to ${selection.kind === "seam" ? "seam" : "part"} ${selection.id}.`
              : "No tracked issues yet. Drag one from Not pinned onto a part or a seam of the map, or use its Pin to… menu."}
          </li>
        )}
      </ul>
      <Backlog backlog={overview.backlog} map={overview.map.map} onPin={onPin} />
      {(overview.unpinned_sessions ?? []).length > 0 && (
        <p className="muted">
          {(overview.unpinned_sessions ?? []).length} session{(overview.unpinned_sessions ?? []).length === 1 ? "" : "s"} on branches outside the convention (unpinned).
        </p>
      )}
    </section>
  );
}
