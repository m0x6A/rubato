import { useEffect, useRef, useState } from "react";
import type { IssueRef, IssueSummary, Me, Overview } from "../../protocol.ts";
import { closedUnmerged, isFinished } from "../../shared/finished.ts";
import { STATE_LABEL, approvalLabel, issueKey, relative } from "../../shared/format.ts";
import { AcceptPlan } from "./AcceptPlan.tsx";
import { DraftSection } from "./DraftSection.tsx";
import { groupPlans } from "./group.ts";
import { MiniMap } from "./MiniMap.tsx";
import { WhereItFits } from "./WhereItFits.tsx";

/** One plan on the board. A plan that exists only as a draft in Rubato (ADR 0053) has no parsed
 * parts in the overview: its row says whose draft it is and whether it fits. */
function Row({ issue, overview, me, done, onSelect, onNotice, accepted, onAccepted }: { issue: IssueSummary; overview: Overview; me: Me | null; done: boolean; onSelect: (i: IssueRef) => void; onNotice: (t: string) => void; accepted: Record<string, string>; onAccepted: (key: string, text: string) => void }) {
  const key = issueKey(issue.issue);
  const plan = issue.plan;
  const draft = issue.draft;
  const draftOnly = !plan && !issue.fit && draft;
  const seams = [...new Set((plan?.affected_parts ?? []).flatMap((p) => p.seams ?? []))];
  const models = plan?.data_models?.length ?? 0;
  const source = issue.pins.length === 0 ? "found by its branch" : null;
  const fits = issue.fit ? issue.fit.fits : draftOnly ? draft.fits : undefined;
  return (
    <li className="plan-row" data-testid={`plan-row-${issue.issue.number}`}>
      <div className="row">
        {plan && <MiniMap map={overview.map.map} layout={overview.layout} plan={plan} />}
        <span className="issue-main">
          <button type="button" className="link" onClick={() => onSelect(issue.issue)}>
            {issueKey(issue.issue)}
          </button>{" "}
          <span className="muted">{issue.title ?? plan?.title ?? ""}</span>
          <div className="chips">
            {(plan?.affected_parts ?? []).map((p) => (
              <span key={p.node} className={`chip part ${p.status}`}>
                {p.node}
              </span>
            ))}
            {seams.map((s) => (
              <span key={s} className="chip seam">
                ⇄ {s}
              </span>
            ))}
          </div>
          {draftOnly ? (
            <div className="muted">
              draft · revision {draft.rev} · {draft.owner} · {relative(draft.at)} · on {draft.branch}
            </div>
          ) : (
            <div className="muted">
              {models} data model{models === 1 ? "" : "s"}
              {source ? ` · ${source}` : ""}
              {plan?.git_ref ? ` · on ${plan.git_ref}` : ""}
            </div>
          )}
        </span>
        <span className="row-end">
          {done && <span className={`badge ${closedUnmerged(issue) ? "closed" : "done"}`}>{closedUnmerged(issue) ? "closed" : "merged"}</span>}
          {draftOnly && <span className="badge draft-badge">draft</span>}
          {fits !== undefined && fits !== null && <span className={`badge ${fits ? "fits" : "does-not-fit"}`}>{fits ? "fits" : "does not fit"}</span>}
          {issue.review && <span className="badge reviewed">reviewed</span>}
          {issue.approval && <span className={`badge ${issue.approval.version === "current" ? "approved" : "approval-stale"}`}>{approvalLabel(issue.approval)}</span>}
        </span>
      </div>
      {/* A finished plan is read, not accepted; a draft is approved on the draft, in the plan. */}
      {!done && !draftOnly && (
        <AcceptPlan
          issue={issue}
          me={me}
          said={accepted[key]}
          onDone={(text) => {
            onAccepted(key, text);
            onNotice(text);
          }}
        />
      )}
    </li>
  );
}

function Column({ name, note, testid, empty, issues, ...row }: { name: string; note: string; testid: string; empty: string; issues: IssueSummary[]; overview: Overview; me: Me | null; done: boolean; onSelect: (i: IssueRef) => void; onNotice: (t: string) => void; accepted: Record<string, string>; onAccepted: (key: string, text: string) => void }) {
  return (
    <section className="plan-column">
      <h3>
        {name} ({issues.length})
      </h3>
      <p className="muted">{note}</p>
      <ul className="plain" data-testid={testid}>
        {issues.map((i) => (
          <Row key={issueKey(i.issue)} issue={i} {...row} />
        ))}
        {issues.length === 0 && <li className="muted">{empty}</li>}
      </ul>
    </section>
  );
}

/** Where the plan's file is on GitHub, when Rubato holds no draft of it to show. A merged plan
 * is on the default branch; an open one on its own branch; a closed, unmerged one maybe nowhere. */
function fileUrl(issue: IssueSummary): string | null {
  const plan = issue.plan;
  if (!plan) {
    return null;
  }
  const merged = isFinished(issue) && !closedUnmerged(issue);
  const ref = merged ? "HEAD" : isFinished(issue) ? null : plan.git_ref;
  return ref ? `https://github.com/${issue.issue.repo}/blob/${encodeURIComponent(ref).replaceAll("%2F", "/")}/${plan.path}` : null;
}

/** One plan, read where the plans are: where it fits, and its draft with every revision. */
function PlanDetail({ issue, overview, me, onBack, onOpenIssue, onNotice, onResume }: { issue: IssueSummary; overview: Overview; me: Me | null; onBack: () => void; onOpenIssue: (i: IssueRef) => void; onNotice: (t: string) => void; onResume?: ((issue: IssueRef, branch: string) => void) | undefined }) {
  const key = issueKey(issue.issue);
  const finished = isFinished(issue);
  const approvalsInRubato = Boolean(overview.comment_approvals_until);
  const heading = useRef<HTMLHeadingElement>(null);
  // The board gives way to the plan: put focus on its heading, as the issue detail does.
  useEffect(() => heading.current?.focus(), [key]);
  const url = fileUrl(issue);
  return (
    <section data-testid="plan-detail">
      <p className="plan-detail-nav">
        <button type="button" onClick={onBack}>
          ← all plans
        </button>{" "}
        <button type="button" data-testid="plan-open-issue" onClick={() => onOpenIssue(issue.issue)} title="The issue: its sessions, pins and room">
          Open issue →
        </button>
      </p>
      <h2 ref={heading} tabIndex={-1}>
        {key}
      </h2>
      {(issue.title ?? issue.plan?.title) && <p>{issue.title ?? issue.plan?.title}</p>}
      <p>
        <span className={`badge ${issue.progress.state}`}>{STATE_LABEL[issue.progress.state]}</span>{" "}
        {issue.closed && (
          <span className="badge closed" title="Closed on GitHub">
            closed
          </span>
        )}
        {finished && (
          <span className="muted" data-testid="plan-finished-note">
            {" "}
            Finished: kept to read, with nothing to accept or approve.
          </span>
        )}
      </p>
      {issue.draft && <DraftSection issue={issue} me={me} approvalsInRubato={approvalsInRubato} onResume={onResume ? (branch) => onResume(issue.issue, branch) : undefined} readOnly={finished} />}
      {(issue.plan || issue.fit) && (
        <>
          <h3 style={{ marginTop: 12 }}>Plan · {issue.plan?.path ?? "does not parse"}</h3>
          <WhereItFits issue={issue} map={overview.map.map} me={me} onNotice={onNotice} approvalsInRubato={approvalsInRubato} readOnly={finished} />
        </>
      )}
      {!issue.draft && issue.plan && (
        <p className="muted" data-testid="plan-file-note">
          Rubato holds no draft of this plan, so its text is not here.{" "}
          {url ? (
            <a href={url} target="_blank" rel="noreferrer">
              Read {issue.plan.path} on GitHub
            </a>
          ) : (
            <>It was at {issue.plan.path} on its branch.</>
          )}
        </p>
      )}
    </section>
  );
}

interface Props {
  overview: Overview;
  me: Me | null;
  /** The issue selected in the shell: with a plan, the board shows that plan instead. */
  selected: IssueSummary | null;
  /** Select an issue (the map paints its plan), or none to come back to the board. */
  onSelect: (issue: IssueRef | null) => void;
  /** Open the issue in the Issues tab. */
  onOpenIssue: (issue: IssueRef) => void;
  onNotice: (text: string) => void;
  /** Resume one of the caller's drafts; absent with no companion of theirs connected. */
  onResume?: ((issue: IssueRef, branch: string) => void) | undefined;
}

/**
 * The Plans tab as a board (#265): plans of issues that are open on GitHub, Proposed and Open,
 * side by side when the panel is wide. Finished plans are not shown until asked for. A plan opens
 * here, not in the Issues tab.
 */
export function PlansBoard({ overview, me, selected, onSelect, onOpenIssue, onNotice, onResume }: Props) {
  const [view, setView] = useState<"open" | "done">("open");
  // What Accept plan said, by issue: an accepted plan's row moves from Proposed to Open, and
  // takes the sentence with it (issue #283).
  const [accepted, setAccepted] = useState<Record<string, string>>({});
  const { proposed, open, done } = groupPlans(overview);
  if (selected && (selected.plan || selected.fit || selected.draft)) {
    return <PlanDetail issue={selected} overview={overview} me={me} onBack={() => onSelect(null)} onOpenIssue={onOpenIssue} onNotice={onNotice} onResume={onResume} />;
  }
  const unfit = [...proposed, ...open].filter((i) => i.fit && !i.fit.fits).length;
  const row = { overview, me, onSelect, onNotice, accepted, onAccepted: (key: string, text: string) => setAccepted((a) => ({ ...a, [key]: text })) };
  return (
    <section className="plans-board" data-testid="plans-panel">
      <div className="big-counts" aria-hidden="true">
        <span>
          <b>{proposed.length}</b> proposed
        </span>
        <span>
          <b>{open.length}</b> open
        </span>
        <span className={unfit > 0 ? "warn" : ""}>
          <b>{unfit}</b> do not fit
        </span>
      </div>
      <div className="lens view-switch" role="group" aria-label="Which plans to list">
        <button type="button" className={view === "open" ? "active" : ""} aria-pressed={view === "open"} data-testid="plans-view-open" onClick={() => setView("open")}>
          Open ({proposed.length + open.length})
        </button>
        <button type="button" className={view === "done" ? "active" : ""} aria-pressed={view === "done"} data-testid="plans-view-done" onClick={() => setView("done")}>
          Done ({done.length})
        </button>
      </div>
      {view === "open" ? (
        <div className="plan-columns" data-testid="plan-columns">
          <Column name="Proposed" note="A plan with no counted approval. An approval counts only while the plan fits the map." testid="plans-proposed" empty="No proposed plans." issues={proposed} done={false} {...row} />
          <Column name="Open" note="Accepted and not merged yet." testid="plans-open" empty="No open plans." issues={open} done={false} {...row} />
        </div>
      ) : (
        <Column name="Done" note="Plans of issues that merged or were closed on GitHub, newest first. Kept to read." testid="plans-done" empty="No finished plans." issues={done} done {...row} />
      )}
    </section>
  );
}
