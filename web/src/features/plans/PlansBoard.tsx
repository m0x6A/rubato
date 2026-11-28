import type { IssueRef, IssueSummary, Overview } from "../../protocol.ts";
import { closedUnmerged, isFinished } from "../../shared/finished.ts";
import { approvalLabel, issueKey, relative } from "../../shared/format.ts";
import { groupPlans } from "./group.ts";
import { MiniMap } from "./MiniMap.tsx";

/** One plan on the board. A plan that exists only as a draft in Rubato (ADR 0053) has no parsed
 * parts in the overview: its row says whose draft it is and whether it fits. */
function Row({ issue, overview, done, onOpen }: { issue: IssueSummary; overview: Overview; done: boolean; onOpen: (i: IssueRef) => void }) {
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
          <button type="button" className="link" onClick={() => onOpen(issue.issue)}>
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
          {/* What calls for something: a plan that does not fit, an approval that no longer covers it. The plan says the rest. */}
          {fits === false && <span className="badge does-not-fit">does not fit</span>}
          {issue.approval && issue.approval.version !== "current" && <span className="badge approval-stale">{approvalLabel(issue.approval)}</span>}
        </span>
      </div>
    </li>
  );
}

function Column({ name, note, testid, empty, issues, ...row }: { name: string; note: string; testid: string; empty: string; issues: IssueSummary[]; overview: Overview; done: boolean; onOpen: (i: IssueRef) => void }) {
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

/** Where to read a plan's text when Rubato holds no draft of it: on GitHub, when the file is still there. */
export function PlanFileNote({ issue }: { issue: IssueSummary }) {
  if (issue.draft || !issue.plan) {
    return null;
  }
  const url = fileUrl(issue);
  return (
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
  );
}

interface Props {
  overview: Overview;
  /** Whether the finished plans show instead of the board; the shell keeps it in the address. */
  done: boolean;
  onDone: (done: boolean) => void;
  /** Open a plan: the Plan part of its issue. */
  onOpen: (issue: IssueRef) => void;
}

/**
 * The plans as a board (#265, issue #357): which plans wait for a person. Plans of issues that
 * are open on GitHub, Proposed and Open, side by side when the panel is wide; finished ones on
 * request. A row says what the plan touches and whether something is wrong with it; the plan is
 * read, approved and accepted in its issue, one click away.
 */
export function PlansBoard({ overview, done: showDone, onDone, onOpen }: Props) {
  const view = showDone ? "done" : "open";
  const setView = (v: "open" | "done") => onDone(v !== "open");
  const { proposed, open, done } = groupPlans(overview);
  const row = { overview, onOpen };
  return (
    <section className="plans-board" data-testid="plans-panel">
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
