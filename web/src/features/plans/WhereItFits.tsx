import type { IssueSummary, Me, SystemMap } from "../../protocol.ts";
import { AcceptPlan } from "./AcceptPlan.tsx";
import { ImportPlan } from "./ImportPlan.tsx";
import { fitReasonText, modelsByPart } from "./group.ts";
import { liveMap } from "../../shared/map.ts";

/** Where a plan fits the system (issue #80): the whole map in small with the plan's parts and
 * seams lit, the parts it affects and what changes in each, the data models it introduces, and
 * whether it fits (ADR 0029). The big map shows the same plan in its plan lens. */
export function WhereItFits({
  issue,
  map: full,
  me,
  onNotice,
  approvalsInRubato = false,
  readOnly = false,
  partsTable = true,
}: {
  issue: IssueSummary;
  map: SystemMap;
  me: Me | null;
  onNotice: (text: string) => void;
  /** The cutover is set: plans are approved in Rubato, on the draft, not by comment (ADR 0054). */
  approvalsInRubato?: boolean;
  /** The issue is finished: where the plan fitted, with no Accept and no Import (#265). */
  readOnly?: boolean;
  /** The Parts affected table; left out where the page lists the plan's changes already (the issue detail). */
  partsTable?: boolean;
}) {
  // Retired parts are drawn nowhere (ADR 0077).
  const map = liveMap(full);
  const plan = issue.plan;
  const fit = issue.fit;
  const status = new Map((plan?.affected_parts ?? []).map((p) => [p.node, p.status]));
  const touched = new Set((plan?.affected_parts ?? []).flatMap((p) => p.seams ?? []));
  const groups = modelsByPart(issue);
  return (
    <section className="where-it-fits" data-testid="where-it-fits">
      <h3 style={{ marginTop: 12 }}>Where it fits</h3>
      <p>
        {fit ? (
          <span className={`badge ${fit.fits ? "fits" : "does-not-fit"}`} data-testid="fit-verdict">
            {fit.fits ? "fits the map" : "does not fit the map"}
          </span>
        ) : (
          <span className="muted">no plan file yet</span>
        )}
      </p>
      {fit && !fit.fits && (
        <ul className="plain warn" data-testid="fit-reasons">
          {(fit.reasons ?? []).map((r, i) => (
            <li key={i}>⚠ {fitReasonText(r)}</li>
          ))}
        </ul>
      )}
      <div className="system-strip" data-testid="system-strip" aria-label="The system map, with this plan's parts lit">
        {/* The plan's parts first; the rest of the map folds behind one line. */}
        {map.nodes
          .filter((n) => status.has(n.id))
          .map((n) => (
            <span key={n.id} className={`strip-part ${status.get(n.id)}`} title={`${n.id}: ${status.get(n.id)}`}>
              {n.id}
            </span>
          ))}
        {(map.seams ?? [])
          .filter((s) => touched.has(s.id))
          .map((s) => (
            <span key={s.id} className="strip-seam" title={`${s.from} → ${s.to}`}>
              ⇄ {s.id}
            </span>
          ))}
        {map.nodes.some((n) => !status.has(n.id)) && (
          <details className="strip-rest">
            <summary>+{map.nodes.filter((n) => !status.has(n.id)).length} parts it leaves alone</summary>
            {map.nodes
              .filter((n) => !status.has(n.id))
              .map((n) => (
                <span key={n.id} className="strip-part untouched" title={n.id}>
                  {n.id}
                </span>
              ))}
          </details>
        )}
      </div>
      {plan && (
        <>
          {partsTable && <h4>Parts affected</h4>}
          {partsTable && (
            <table className="mini" data-testid="parts-affected">
              <thead>
                <tr>
                  <th>Part</th>
                  <th>Status</th>
                  <th>What changes</th>
                  <th>Seams</th>
                </tr>
              </thead>
              <tbody>
                {plan.affected_parts.map((p) => (
                  <tr key={p.node}>
                    <td>{p.node}</td>
                    <td>{p.status}</td>
                    <td>{p.change}</td>
                    <td className="muted">{(p.seams ?? []).join(", ")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <h4>Data models</h4>
          {groups.length === 0 ? (
            <p className="muted" data-testid="no-data-models">
              The plan introduces no data models.
            </p>
          ) : (
            <dl className="data-models" data-testid="data-models">
              {groups.map((g) => (
                <div key={g.part}>
                  <dt>{g.part}</dt>
                  {g.models.map((m, i) => (
                    <dd key={i}>
                      <span className={`badge model-${m.change}`}>{m.change}</span> <code>{m.model}</code> <span className="muted">{m.kind}</span>
                      {m.crosses && <span className="muted"> · crosses {m.crosses}</span>}
                      {m.note && <span className="muted"> · {m.note}</span>}
                    </dd>
                  ))}
                </div>
              ))}
            </dl>
          )}
        </>
      )}
      {readOnly ? null : issue.outside_rubato ? (
        <ImportPlan issue={issue} me={me} onDone={onNotice} />
      ) : approvalsInRubato ? (
        issue.approval?.version !== "current" && <p className="muted" data-testid="approve-in-rubato">Plans are approved in Rubato on this server: approve the draft above.</p>
      ) : (
        <AcceptPlan issue={issue} me={me} onDone={onNotice} />
      )}
    </section>
  );
}
