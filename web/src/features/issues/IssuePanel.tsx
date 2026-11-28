import { Fragment, type ReactNode, useEffect, useRef, useState } from "react";
import { SpendGauge, SpendSparkline, StageRail } from "./visuals.tsx";
import { SessionChat } from "../chat/index.ts";
import { AgentLineView, agentLineFor } from "../companions/index.ts";
import { PinMenu } from "../pins/index.ts";
import { DraftSection, WhereItFits } from "../plans/index.ts";
import { PullView } from "../pulls/index.ts";
import { RoomSection, type VoiceHeld, roomFor, roomLimit } from "../room/index.ts";
import type { IssueRef, IssueSummary, MapRef, Me, Overview, PinRequest } from "../../protocol.ts";
import { isFinished } from "../../shared/finished.ts";
import { STATE_LABEL, issueKey, relative } from "../../shared/format.ts";
import { isRetired } from "../../shared/map.ts";
import type { IssueSection } from "../../shared/route.ts";
import { nextAction } from "./next.ts";
import { ownConnectedCompanions } from "./own.ts";
import { PlanSignals } from "./PlanSignals.tsx";

interface Props {
  overview: Overview;
  me: Me | null;
  /** The issue shown. */
  selected: IssueSummary;
  /** Which part of the issue shows; the others stay as they were left. */
  section: IssueSection;
  onSection: (section: IssueSection) => void;
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
  /** Opens Companions, where a companion is paired: the way to "Start session". */
  onPairCompanion?: (() => void) | undefined;
  /** What the plan says in the map's terms, drawn by the map's own feature; shown with the plan. */
  planOnMap?: ReactNode;
}

const SECTIONS: { id: IssueSection; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "plan", label: "Plan" },
  { id: "room", label: "Room" },
  { id: "sessions", label: "Sessions" },
  { id: "pr", label: "Pull request" },
];

/**
 * One issue (issue #357): a header that says where it stands and offers the one next step, then
 * its parts, one at a time: Overview, Plan, Room, Sessions, Pull request. The parts not showing
 * stay mounted, so what a person typed in the room or to a session is there when they come back,
 * and a voice channel they joined stays joined until the issue closes. Only the pull request is
 * read when its section is first opened (ADR 0034).
 */
export function IssuePanel({ overview, me, selected, section, onSection, onClose, onOpenIssue, onLaunch, onPin, onUnpin, onNotice, onShowSeam, onLaunchLead, onPairCompanion, planOnMap }: Props) {
  const own = ownConnectedCompanions(overview, me);
  // A lead only with the server's flag on, on a companion of yours that runs leads (ADR 0072).
  const leadOn = overview.orchestrator ? own.find((c) => c.can_lead) : undefined;
  // The list gives way to the issue, and a row that was just pinned leaves "Not pinned" with the
  // button that had focus: put focus on the issue's heading when it opens.
  const heading = useRef<HTMLHeadingElement>(null);
  const key = issueKey(selected.issue);
  useEffect(() => {
    heading.current?.focus();
  }, [key]);
  // The pull request is read from GitHub once its section has been opened, and then kept.
  const [pullSeen, setPullSeen] = useState(false);
  useEffect(() => setPullSeen(false), [key]);
  useEffect(() => {
    if (section === "pr") {
      setPullSeen(true);
    }
  }, [section]);
  // The room's voice channel, when this person is in it: said in the header on every section.
  const [voice, setVoice] = useState<VoiceHeld | null>(null);

  const finished = isFinished(selected);
  const next = nextAction(selected, overview, me);
  const sessions = selected.sessions ?? [];
  const pr = selected.pull_request;
  const hasRoom = roomFor(selected) !== null;
  const shown = SECTIONS.filter((s) => s.id !== "pr" || pr);
  // A link may name a section this issue does not have (no pull request yet): the overview shows.
  const at = shown.some((s) => s.id === section) ? section : "overview";
  const count: Partial<Record<IssueSection, number>> = { sessions: sessions.length };

  return (
    <section data-testid="issue-detail" className="issue-view">
      <header className="issue-head">
        <button onClick={onClose}>← all issues</button>
        <h2 ref={heading} tabIndex={-1}>
          {selected.url ? (
            <a href={selected.url} target="_blank" rel="noreferrer">
              {key}
            </a>
          ) : (
            key
          )}
        </h2>
        {selected.title && <p className="issue-title">{selected.title}</p>}
        <StageRail state={selected.progress.state} blockedFrom={selected.progress.blocked_from} />
        <p className="issue-state">
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
        ) : (
          <div className="issue-next" data-testid="issue-next" data-next={next.kind}>
            {next.says && (
              <span className="muted" data-testid={next.kind === "pair" || (next.kind === "none" && own.length === 0) ? "no-companion" : "next-says"}>
                {next.says}
              </span>
            )}
            {next.kind === "section" && (
              <button type="button" className="primary" data-testid="next-step" onClick={() => onSection(next.section)}>
                {next.label}
              </button>
            )}
            {next.kind === "pin" && (
              <span className="primary-menu" data-testid="next-step">
                <PinMenu map={overview.map.map} issue={selected.issue} pinned={selected.pins} onPin={onPin} />
              </span>
            )}
            {next.kind === "pair" && onPairCompanion && (
              <button type="button" className="primary" data-testid="next-step" onClick={onPairCompanion}>
                Pair a companion
              </button>
            )}
            {next.kind === "resume" && (
              <button type="button" className="primary" data-testid="next-step" onClick={() => onLaunch(selected.issue, next.branch)} title={`start a session in your companion ${own[0]?.hostname ?? ""} on the draft's branch`}>
                Resume on {next.branch}
              </button>
            )}
            {next.kind === "start" && (
              <>
                <button type="button" className="primary" data-testid="start-session" onClick={() => onLaunch(selected.issue)} title={`start in your companion ${own[0]?.hostname ?? ""}`}>
                  Start session
                </button>
                {leadOn && onLaunchLead && (
                  <button
                    type="button"
                    className="link"
                    data-testid="start-lead"
                    onClick={() => onLaunchLead(selected.issue, leadOn.companion_id)}
                    title={`a lead agent on ${leadOn.hostname}: it starts, messages and stops only the workers it launches for you; plans still wait for a person`}
                  >
                    or start as lead agent
                  </button>
                )}
              </>
            )}
          </div>
        )}
        {voice && (
          <p className="in-voice" data-testid="in-voice">
            <span className="dot live" aria-hidden="true" /> In voice{voice.via ? ` · speech via ${voice.via}` : ""}{" "}
            <button type="button" onClick={voice.leave}>
              Leave
            </button>
          </p>
        )}
      </header>

      <nav className="tabs sections" aria-label="Parts of this issue">
        {shown.map((s) => (
          <button key={s.id} type="button" className={at === s.id ? "active" : ""} aria-current={at === s.id ? "page" : undefined} data-testid={s.id === "pr" ? "open-pull-view" : `section-${s.id}`} onClick={() => onSection(s.id)}>
            {s.label}
            {(count[s.id] ?? 0) > 0 && (
              <span className="tab-count" aria-label={`${count[s.id]}`}>
                {count[s.id]}
              </span>
            )}
          </button>
        ))}
      </nav>

      <div className="issue-section" data-section="overview" hidden={at !== "overview"}>
        {/* The gauge once something was spent or a session ran; before that, one line says the cap. */}
        <div className="instruments">
          {(selected.spend_usd ?? 0) > 0 || sessions.length > 0 || (selected.spend_history ?? []).length > 0 ? (
            <SpendGauge spend={selected.spend_usd ?? 0} cap={selected.plan?.spend_cap_usd} />
          ) : selected.plan?.spend_cap_usd ? (
            <span className="muted spend-cap-note">spend cap ${selected.plan.spend_cap_usd.toFixed(0)} · nothing spent yet</span>
          ) : null}
          <SpendSparkline points={selected.spend_history ?? []} />
        </div>
        <h3>Pinned to</h3>
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
        {/* With no pin, pinning is the header's next step; here it adds another. */}
        {selected.pins.length > 0 && (
          <p>
            <PinMenu map={overview.map.map} issue={selected.issue} pinned={selected.pins} onPin={onPin} />
          </p>
        )}
        {(selected.plan || selected.review || selected.approval) && <PlanSignals issue={selected} />}
        {(selected.decisions ?? []).length > 0 && (
          <>
            <h3>Decisions</h3>
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
      </div>

      <div className="issue-section" data-section="plan" hidden={at !== "plan"}>
        {selected.draft && (
          <DraftSection issue={selected} me={me} approvalsInRubato={Boolean(overview.comment_approvals_until)} onResume={own.length > 0 ? (branch) => onLaunch(selected.issue, branch) : undefined} readOnly={finished} />
        )}
        {(selected.plan || selected.fit) && (
          <>
            <h3>Plan · {selected.plan?.path ?? "does not parse"}</h3>
            <WhereItFits issue={selected} map={overview.map.map} me={me} partsTable={!selected.plan} onNotice={(t) => onNotice?.(t)} approvalsInRubato={Boolean(overview.comment_approvals_until)} readOnly={finished} />
            {selected.plan?.assumed && selected.plan.assumed.some((a) => a.scope === "seam") && (
              <p className="warn">⚠ {selected.plan.assumed.filter((a) => a.scope === "seam").length} seam-level question(s) wait for a human.</p>
            )}
          </>
        )}
        {planOnMap}
        {!selected.draft && !selected.plan && !selected.fit && (
          <p className="muted" data-testid="no-plan">
            No plan yet. A session on this issue writes it, and it shows here as it is drafted.
          </p>
        )}
      </div>

      <div className="issue-section" data-section="room" hidden={at !== "room"}>
        {/* The plan's room: the draft's branch, else the plan's; nothing without either (ADR 0057). */}
        <RoomSection
          issue={selected}
          me={me}
          maxChars={roomLimit(overview)}
          onOpenIssue={onOpenIssue}
          canOpen={(i) => overview.issues.some((x) => x.issue.repo === i.repo && x.issue.number === i.number)}
          onShowSeam={onShowSeam}
          onVoice={setVoice}
        />
        {!hasRoom && (
          <p className="muted" data-testid="no-room">
            The room opens with the plan: once a plan has a branch, the team talks about it here.
          </p>
        )}
      </div>

      <div className="issue-section" data-section="sessions" hidden={at !== "sessions"}>
        {sessions.length > 0 ? (
          <table className="mini">
            <tbody>
              {sessions.map((s, i) => (
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
        ) : (
          <p className="muted" data-testid="no-sessions">
            No session yet.{finished ? "" : " One starts from the top of this issue, in your own companion."}
          </p>
        )}
      </div>

      {pr && (
        <div className="issue-section" data-section="pr" hidden={at !== "pr"}>
          <p>
            PR{" "}
            <a href={pr.url} target="_blank" rel="noreferrer">
              #{pr.number}
            </a>{" "}
            <span className="muted">
              {pr.merged_at ? "merged" : pr.open ? "open" : "closed"} · checks {pr.checks}
            </span>
          </p>
          {pullSeen && <PullView repo={pr.repo} number={pr.number} refreshKey={overview.generated_at} />}
        </div>
      )}
    </section>
  );
}
