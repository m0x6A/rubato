import type { IssueRef, IssueSummary, Me, Overview } from "../../protocol.ts";
import { approvalLabel, issueKey } from "../../shared/format.ts";
import { AcceptPlan } from "./AcceptPlan.tsx";
import { groupPlans } from "./group.ts";
import { MiniMap } from "./MiniMap.tsx";

function Row({ issue, overview, me, onSelect, onNotice }: { issue: IssueSummary; overview: Overview; me: Me | null; onSelect: (i: IssueRef) => void; onNotice: (t: string) => void }) {
  const plan = issue.plan;
  const seams = [...new Set((plan?.affected_parts ?? []).flatMap((p) => p.seams ?? []))];
  const models = plan?.data_models?.length ?? 0;
  const source = issue.pins.length === 0 ? "found by its branch" : null;
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
          <div className="muted">
            {models} data model{models === 1 ? "" : "s"}
            {source ? ` · ${source}` : ""}
            {plan?.git_ref ? ` · on ${plan.git_ref}` : ""}
          </div>
        </span>
        <span className="row-end">
          {issue.fit && <span className={`badge ${issue.fit.fits ? "fits" : "does-not-fit"}`}>{issue.fit.fits ? "fits" : "does not fit"}</span>}
          {issue.review && <span className="badge reviewed">reviewed</span>}
          {issue.approval && <span className={`badge ${issue.approval.version === "current" ? "approved" : "approval-stale"}`}>{approvalLabel(issue.approval)}</span>}
        </span>
      </div>
      <AcceptPlan issue={issue} me={me} onDone={onNotice} />
    </li>
  );
}

/** The Plans tab (issue #80): proposed and open plans, each with where it fits. */
export function PlansPanel({ overview, me, onSelect, onNotice }: { overview: Overview; me: Me | null; onSelect: (i: IssueRef) => void; onNotice: (t: string) => void }) {
  const { proposed, open } = groupPlans(overview);
  return (
    <section data-testid="plans-panel">
      <div className="big-counts" aria-hidden="true">
        <span>
          <b>{proposed.length}</b> proposed
        </span>
        <span>
          <b>{open.length}</b> open
        </span>
        <span className={[...proposed, ...open].some((i) => i.fit && !i.fit.fits) ? "warn" : ""}>
          <b>{[...proposed, ...open].filter((i) => i.fit && !i.fit.fits).length}</b> do not fit
        </span>
      </div>
      <h3>Proposed ({proposed.length})</h3>
      <p className="muted">A plan with no counted approval. An approval counts only while the plan fits the map.</p>
      <ul className="plain" data-testid="plans-proposed">
        {proposed.map((i) => (
          <Row key={issueKey(i.issue)} issue={i} overview={overview} me={me} onSelect={onSelect} onNotice={onNotice} />
        ))}
        {proposed.length === 0 && <li className="muted">No proposed plans.</li>}
      </ul>
      <h3 style={{ marginTop: 12 }}>Open ({open.length})</h3>
      <p className="muted">Accepted and not merged yet.</p>
      <ul className="plain" data-testid="plans-open">
        {open.map((i) => (
          <Row key={issueKey(i.issue)} issue={i} overview={overview} me={me} onSelect={onSelect} onNotice={onNotice} />
        ))}
        {open.length === 0 && <li className="muted">No open plans.</li>}
      </ul>
    </section>
  );
}
