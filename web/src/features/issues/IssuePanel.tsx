import { Fragment, useEffect, useRef, useState } from "react";
import { ProgressRing, SpendGauge, SpendSparkline, StageRail } from "./visuals.tsx";
import { SessionChat } from "../chat/index.ts";
import { AgentLineView, agentLineFor } from "../companions/index.ts";
import { PinMenu } from "../pins/index.ts";
import { DraftSection, WhereItFits } from "../plans/index.ts";
import { PullView } from "../pulls/index.ts";
import { QueueMenu } from "../queue/index.ts";
import { RoomSection, roomLimit } from "../room/index.ts";
import type { IssueRef, IssueSummary, MapRef, Me, Overview, PinRequest } from "../../protocol.ts";
import { setIssueDrag } from "../../shared/dnd.ts";
import { closedUnmerged, finishedIssues, isFinished, openIssues } from "../../shared/finished.ts";
import { STATE_LABEL, issueKey, relative } from "../../shared/format.ts";
import { Backlog } from "./Backlog.tsx";
import { PlanSignals, SignalBadges } from "./PlanSignals.tsx";
import { isRetired } from "../../shared/map.ts";

interface Props {
  overview: Overview;
  me: Me | null;
  selected: IssueSummary | null;
  /** The part or seam selected on the map; the list shows the issues pinned to exactly it. */
  selection: MapRef | null;
  onSelect: (issue: IssueRef | null) => void;
  onClearSelection: () => void;
  /** `branch` resumes one of the caller's drafts on it (ADR 0053). */
  onLaunch: (issue: IssueRef, branch?: string) => void;
  /** Adds a pin; an issue may have several. The keyboard twin of dropping a row on the map. */
  onPin: (issue: IssueRef, target: MapRef) => void;
  onUnpin: (req: PinRequest) => void;
  /** Queue an issue on one of the caller's own companions. Without it no "Queue on…" is offered. */
  onQueue?: ((issue: IssueRef, companionId: string) => void) | undefined;
  /** Says what *Accept plan* did. */
  onNotice?: ((text: string) => void) | undefined;
  /** Show a seam on the map, from a room's seam notice. */
  onShowSeam?: ((seam: string) => void) | undefined;
  /** Start your own lead agent on the issue (ADR 0072): offered only with the orchestrator on. */
  onLaunchLead?: ((issue: IssueRef, companionId: string) => void) | undefined;
  /** Opens the Companions tab, where a companion is paired: the way to "Start session". */
  onPairCompanion?: (() => void) | undefined;
}

/** Companions the current user may queue issues on: their own, paired, not revoked (ADR 0020). */
export function ownLanes(overview: Overview, me: Me | null) {
  if (!me) {
    return [];
  }
  return overview.companions.filter((c) => c.user === me.login && !c.revoked_at);
}

/** Companions the current user may start sessions in: their own, paired, connected (ADR 0004). */
export function ownConnectedCompanions(overview: Overview, me: Me | null) {
  if (!me) {
    return [];
  }
  return overview.companions.filter((c) => c.user === me.login && c.connected && !c.revoked_at);
}

const ORDER = ["blocked", "unknown", "building", "designing", "designed", "started", "in_review", "planned", "done"] as const;

export function IssuePanel({ overview, me, selected, selection, onSelect, onClearSelection, onLaunch, onPin, onUnpin, onQueue, onNotice, onShowSeam, onLaunchLead, onPairCompanion }: Props) {
  const own = ownConnectedCompanions(overview, me);
  // A lead only with the server's flag on, on a companion of yours that runs leads (ADR 0072).
  const leadOn = overview.orchestrator ? own.find((c) => c.can_lead) : undefined;
  // The name of this workspace, when there are several: what "counts in" is told apart from.
  const hereName = (overview.workspaces ?? []).find((w) => w.id === overview.workspace)?.name;
  const lanes = onQueue ? ownLanes(overview, me) : [];
  // The list gives way to the detail, and a row that was just pinned leaves "Not pinned" with the
  // button that had focus: put focus on the detail's heading when an issue opens.
  const heading = useRef<HTMLHeadingElement>(null);
  const openKey = selected ? issueKey(selected.issue) : null;
  useEffect(() => {
    if (openKey) {
      heading.current?.focus();
    }
  }, [openKey]);
  // The pull request's view, opened from its line; closed again when another issue opens.
  const [pullOpen, setPullOpen] = useState(false);
  useEffect(() => setPullOpen(false), [openKey]);
  // Open on GitHub by default; the finished ones on request, newest first (#265).
  const [view, setView] = useState<"open" | "done">("open");
  const pinnedHere = (i: IssueSummary) => !selection || i.pins.some((p) => p.kind === selection.kind && p.id === selection.id);
  const open = openIssues(overview)
    .filter(pinnedHere)
    .sort((a, b) => ORDER.indexOf(a.progress.state) - ORDER.indexOf(b.progress.state) || a.issue.number - b.issue.number);
  const done = finishedIssues(overview).filter(pinnedHere);
  const issues = view === "done" ? done : open;

  if (selected) {
    const key = issueKey(selected.issue);
    const finished = isFinished(selected);
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
        <div className="instruments">
          <StageRail state={selected.progress.state} blockedFrom={selected.progress.blocked_from} />
          {/* The gauge once something was spent or a session ran; before that, one line says the cap. */}
          {(selected.spend_usd ?? 0) > 0 || (selected.sessions ?? []).length > 0 || (selected.spend_history ?? []).length > 0 ? (
            <SpendGauge spend={selected.spend_usd ?? 0} cap={selected.plan?.spend_cap_usd} />
          ) : selected.plan?.spend_cap_usd ? (
            <span className="muted spend-cap-note">spend cap ${selected.plan.spend_cap_usd.toFixed(0)} · nothing spent yet</span>
          ) : null}
          <SpendSparkline points={selected.spend_history ?? []} />
        </div>
        <p>
          <span className={`badge ${selected.progress.state}`}>{STATE_LABEL[selected.progress.state]}</span>{" "}
          {selected.closed && (
            <span className="badge closed" data-testid="closed-badge" title="Closed on GitHub">
              closed
            </span>
          )}{" "}
          <span className="muted">last signal {relative(selected.progress.last_seen)}</span>
          {selected.progress.reason && <span className="muted"> · {selected.progress.reason}</span>}
        </p>
        {finished ? (
          <p className="muted" data-testid="finished-note">
            {selected.progress.state === "done" ? "Merged." : "Closed on GitHub."} Finished: nothing starts from here.
          </p>
        ) : null}
        {finished ? null : own.length > 0 ? (
          <button className="primary" data-testid="start-session" onClick={() => onLaunch(selected.issue)} disabled={selected.pins.length === 0} title={selected.pins.length === 0 ? "pin the issue to the map first" : `start in your companion ${own[0]?.hostname ?? ""}`}>
            Start session in my tmux
          </button>
        ) : null}
        {!finished && own.length > 0 && leadOn && onLaunchLead ? (
          <button
            data-testid="start-lead"
            onClick={() => onLaunchLead(selected.issue, leadOn.companion_id)}
            disabled={selected.pins.length === 0}
            title={`a lead agent on ${leadOn.hostname}: it starts, messages and stops only the workers it launches for you; plans still wait for a person`}
          >
            Start as lead agent
          </button>
        ) : null}
        {finished || own.length > 0 ? null : (
          <p className="muted" data-testid="no-companion">
            To start a session on this issue, connect a companion of yours: run <code>rubato-companion run</code> on your machine
            {onPairCompanion && (
              <>
                , or{" "}
                <button type="button" className="link" onClick={onPairCompanion}>
                  pair one first
                </button>
              </>
            )}
            .
          </p>
        )}
        <h3 style={{ marginTop: 12 }}>Pinned to</h3>
        <ul className="plain">
          {selected.pins.map((p) => (
            <li key={`${p.kind}:${p.id}`} className="issue-row">
              <span>
                <span className="muted">{p.kind}</span> {p.id}
                {/* Still pinned, but the part is drawn and offered nowhere (ADR 0077). */}
                {isRetired(overview.map.map, p.id) && <span className="muted"> (retired)</span>}
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
                  <Fragment key={s.session_id ?? i}>
                    <tr>
                      <td>
                        <span className={`badge ${s.status}`}>{s.status}</span>
                      </td>
                      <td className="muted">{s.source}</td>
                      <td>{s.tmux?.session ?? s.cwd}</td>
                      <td className="muted">{relative(s.last_seen)}</td>
                      <td>{s.spend_usd !== undefined && s.spend_usd !== null ? `$${s.spend_usd.toFixed(2)}` : ""}</td>
                    </tr>
                    {/* What its agent says it is doing (ADR 0071): a signal with its age, never a state. */}
                    {(() => {
                      const line = agentLineFor(overview.agent_lines, s);
                      return (
                        line && (
                          <tr className="agent-line-row">
                            <td colSpan={5}>
                              <AgentLineView line={line} session={s} />
                            </td>
                          </tr>
                        )
                      );
                    })()}
                    {/* A session Rubato launched can be messaged by its owner and read by everyone (ADR 0051). */}
                    {s.launch_id && (
                      <tr className="chat-row">
                        <td colSpan={5}>
                          <SessionChat session={s} me={me} />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                ))}
              </tbody>
            </table>
          </>
        )}
        {selected.draft && (
          <DraftSection issue={selected} me={me} approvalsInRubato={Boolean(overview.comment_approvals_until)} onResume={own.length > 0 ? (branch) => onLaunch(selected.issue, branch) : undefined} readOnly={finished} />
        )}
        {(selected.plan || selected.fit) && (
          <>
            <h3 style={{ marginTop: 12 }}>Plan · {selected.plan?.path ?? "does not parse"}</h3>
            <WhereItFits issue={selected} map={overview.map.map} me={me} partsTable={!selected.plan} onNotice={(t) => onNotice?.(t)} approvalsInRubato={Boolean(overview.comment_approvals_until)} readOnly={finished} />
            {selected.plan?.assumed && selected.plan.assumed.some((a) => a.scope === "seam") && (
              <p className="warn">⚠ {selected.plan.assumed.filter((a) => a.scope === "seam").length} seam-level question(s) wait for a human.</p>
            )}
          </>
        )}
        {/* The plan's room: the draft's branch, else the plan's; nothing without either (ADR 0057). */}
        <RoomSection
          issue={selected}
          me={me}
          maxChars={roomLimit(overview)}
          onOpenIssue={onSelect}
          canOpen={(i) => overview.issues.some((x) => x.issue.repo === i.repo && x.issue.number === i.number)}
          onShowSeam={onShowSeam}
        />
        {selected.pull_request && (
          <p>
            PR{" "}
            <a href={selected.pull_request.url} target="_blank" rel="noreferrer">
              #{selected.pull_request.number}
            </a>{" "}
            <span className="muted">
              {selected.pull_request.merged_at ? "merged" : selected.pull_request.open ? "open" : "closed"} · checks {selected.pull_request.checks}
            </span>{" "}
            <button type="button" className="link" data-testid="open-pull-view" onClick={() => setPullOpen((v) => !v)}>
              {pullOpen ? "Hide" : "View"}
            </button>
          </p>
        )}
        {selected.pull_request && pullOpen && <PullView repo={selected.pull_request.repo} number={selected.pull_request.number} refreshKey={overview.generated_at} />}
        {(selected.plan || selected.review || selected.approval) && <PlanSignals issue={selected} />}
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
      <div className="lens view-switch" role="group" aria-label="Which issues to list">
        <button type="button" className={view === "open" ? "active" : ""} aria-pressed={view === "open"} data-testid="issues-view-open" onClick={() => setView("open")}>
          Open ({open.length})
        </button>
        <button type="button" className={view === "done" ? "active" : ""} aria-pressed={view === "done"} data-testid="issues-view-done" onClick={() => setView("done")}>
          Done ({done.length})
        </button>
      </div>
      <ul className="plain" data-testid={view === "done" ? "issues-done" : "issues-open"}>
        {issues.map((i) => (
          <li
            key={issueKey(i.issue)}
            className="issue-row"
            onClick={() => onSelect(i.issue)}
            data-testid={`issue-${i.issue.number}`}
            draggable={view === "open"}
            onDragStart={view === "open" ? (e) => setIssueDrag(e, i.issue) : undefined}
            title={view === "open" ? "Drag onto a part or a seam of the map to add a pin" : undefined}
          >
            {/* `issue-row-N` names the whole row for the e2e; `issue-N` is the older id on the same row. */}
            <div className="row" data-testid={`issue-row-${i.issue.number}`}>
              <ProgressRing state={i.progress.state} />
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
                {closedUnmerged(i) && (
                  <span className="badge closed" title="Closed on GitHub without a merged pull request">
                    closed
                  </span>
                )}
                <SignalBadges issue={i} here={hereName} />
                {view === "open" && <PinMenu map={overview.map.map} issue={i.issue} pinned={i.pins} onPin={onPin} />}
                {view === "open" && onQueue && <QueueMenu issue={i.issue} lanes={lanes} queue={overview.queue ?? []} onQueue={onQueue} />}
              </span>
            </div>
          </li>
        ))}
        {issues.length === 0 && (
          <li className="muted" data-testid="no-tracked-issues">
            {view === "done"
              ? selection
                ? `No finished issue is pinned to ${selection.kind === "seam" ? "seam" : "part"} ${selection.id}.`
                : "Nothing finished yet."
              : selection
                ? `No open issue is pinned to ${selection.kind === "seam" ? "seam" : "part"} ${selection.id}.`
                : done.length > 0
                  ? `Nothing open on GitHub is tracked here. ${done.length} finished: see Done. Drag an issue from Not pinned onto the map to track it.`
                  : "No tracked issues yet. Drag one from Not pinned onto a part or a seam of the map, or use its Pin to… menu."}
          </li>
        )}
      </ul>
      {view === "open" && <Backlog backlog={overview.backlog} map={overview.map.map} onPin={onPin} lanes={lanes} queue={overview.queue ?? []} onQueue={onQueue} listedAbove={new Set(open.map((i) => issueKey(i.issue)))} />}
      {view === "open" && (overview.unpinned_sessions ?? []).length > 0 && (
        <p className="muted">
          {(overview.unpinned_sessions ?? []).length} session{(overview.unpinned_sessions ?? []).length === 1 ? "" : "s"} on branches outside the convention (unpinned).
        </p>
      )}
    </section>
  );
}
