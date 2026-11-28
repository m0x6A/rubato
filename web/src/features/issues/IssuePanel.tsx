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
import { ownConnectedCompanions } from "./own.ts";
import { PlanSignals, SignalBadges } from "./PlanSignals.tsx";
import { isRetired } from "../../shared/map.ts";

interface Props {
  overview: Overview;
  me: Me | null;
  /** The issue shown. */
  selected: IssueSummary;
  /** Back to the list. */
  onClose: () => void;
  /** Open another issue: the other plan a room's seam notice names. */
  onOpenIssue: (issue: IssueRef) => void;
  /** `branch` resumes one of the caller's drafts on it (ADR 0053). */
  onLaunch: (issue: IssueRef, branch?: string) => void;
  /** Adds a pin; an issue may have several. The keyboard twin of dropping a row on the map. */
  onPin: (issue: IssueRef, target: MapRef) => void;
  onUnpin: (req: PinRequest) => void;
  /** Says what *Accept plan* did. */
  onNotice?: ((text: string) => void) | undefined;
  /** Show a seam on the map, from a room's seam notice. */
  onShowSeam?: ((seam: string) => void) | undefined;
  /** Start your own lead agent on the issue (ADR 0072): offered only with the orchestrator on. */
  onLaunchLead?: ((issue: IssueRef, companionId: string) => void) | undefined;
  /** Opens the Companions tab, where a companion is paired: the way to "Start session". */
  onPairCompanion?: (() => void) | undefined;
}


export function IssuePanel({ overview, me, selected, onClose, onOpenIssue, onLaunch, onPin, onUnpin, onNotice, onShowSeam, onLaunchLead, onPairCompanion }: Props) {
  const own = ownConnectedCompanions(overview, me);
  // A lead only with the server's flag on, on a companion of yours that runs leads (ADR 0072).
  const leadOn = overview.orchestrator ? own.find((c) => c.can_lead) : undefined;
  // The name of this workspace, when there are several: what "counts in" is told apart from.
  const hereName = (overview.workspaces ?? []).find((w) => w.id === overview.workspace)?.name;
  // The list gives way to the detail, and a row that was just pinned leaves "Not pinned" with the
  // button that had focus: put focus on the detail's heading when an issue opens.
  const heading = useRef<HTMLHeadingElement>(null);
  const openKey = issueKey(selected.issue);
  useEffect(() => {
    if (openKey) {
      heading.current?.focus();
    }
  }, [openKey]);
  // The pull request's view, opened from its line; closed again when another issue opens.
  const [pullOpen, setPullOpen] = useState(false);
  useEffect(() => setPullOpen(false), [openKey]);

  {
    const key = issueKey(selected.issue);
    const finished = isFinished(selected);
    return (
      <section data-testid="issue-detail">
        <button onClick={onClose}>← all issues</button>
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
          onOpenIssue={onOpenIssue}
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
}
