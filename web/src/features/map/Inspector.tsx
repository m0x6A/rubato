import type { Flow, IssueRef, MapRef, Overview } from "../../protocol.ts";
import { STATE_LABEL, issueKey, relative } from "../../shared/format.ts";
import { type FlowLine, type Touching, explain } from "./explain.ts";

interface Props {
  overview: Overview;
  /** The selected part or seam. */
  selection: MapRef;
  onSelect: (ref: MapRef | null) => void;
  onSelectIssue: (issue: IssueRef) => void;
}

/** What nobody has written down yet is said, not left blank: a gap in the shared picture. */
const NOT_DESCRIBED = "not described yet";

function Ref({ target, onSelect, children }: { target: MapRef; onSelect: Props["onSelect"]; children?: string }) {
  return (
    <button type="button" className="link mono" onClick={() => onSelect(target)}>
      {children ?? target.id}
    </button>
  );
}

function FlowList({ lines, direction, onSelect }: { lines: FlowLine[]; direction: "from" | "to"; onSelect: Props["onSelect"] }) {
  if (lines.length === 0) {
    return <p className="muted">Nothing declared.</p>;
  }
  return (
    <ul className="plain">
      {lines.map((l, i) => (
        <li key={`${l.seam}-${i}`}>
          <span className={`flow-kind ${l.flow.kind}`}>{l.flow.kind}</span> {l.flow.carries} <span className="muted">{direction}</span> <Ref target={{ kind: "node", id: l.other }} onSelect={onSelect} />{" "}
          <span className="muted">over</span> <Ref target={{ kind: "seam", id: l.seam }} onSelect={onSelect} />
        </li>
      ))}
    </ul>
  );
}

function SeamFlows({ flows, onSelect }: { flows: Flow[]; onSelect: Props["onSelect"] }) {
  return (
    <ul className="plain">
      {flows.map((f, i) => (
        <li key={i}>
          <span className={`flow-kind ${f.kind}`}>{f.kind}</span> {f.carries}: <Ref target={{ kind: "node", id: f.from }} onSelect={onSelect} /> → <Ref target={{ kind: "node", id: f.to }} onSelect={onSelect} />
        </li>
      ))}
    </ul>
  );
}

function TouchingNow({ touching, onSelectIssue }: { touching: Touching; onSelectIssue: Props["onSelectIssue"] }) {
  const { issues, plans, sessions, decisions } = touching;
  if (issues.length + plans.length + sessions.length + decisions.length === 0) {
    return <p className="muted">No issue is pinned here and no open plan names it.</p>;
  }
  return (
    <>
      {issues.length > 0 && (
        <ul className="plain">
          {issues.map((i) => (
            <li key={issueKey(i.issue)}>
              <button type="button" className="link" onClick={() => onSelectIssue(i.issue)}>
                {issueKey(i.issue)}
              </button>{" "}
              {i.title} <span className={`badge ${i.progress.state}`}>{STATE_LABEL[i.progress.state]}</span>
            </li>
          ))}
        </ul>
      )}
      {plans.length > 0 && (
        <>
          <h4>Open plans</h4>
          <ul className="plain">
            {plans.map((p) => (
              <li key={issueKey(p.issue)}>
                <button type="button" className="link" onClick={() => onSelectIssue(p.issue)}>
                  {issueKey(p.issue)}
                </button>
                {p.status ? ` (${p.status})` : ""}
                {p.change ? `: ${p.change}` : p.title ? `: ${p.title}` : ""}
              </li>
            ))}
          </ul>
        </>
      )}
      {sessions.length > 0 && (
        <>
          <h4>Sessions</h4>
          <ul className="plain">
            {sessions.map((s, i) => (
              <li key={s.session_id ?? i}>
                {s.user} <span className={`badge ${s.status}`}>{s.status}</span> <span className="muted">seen {relative(s.last_seen)}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {decisions.length > 0 && (
        <>
          <h4>Decisions</h4>
          <ul className="plain">
            {decisions.map((d) => (
              <li key={d.comment_id}>
                <a href={d.url} target="_blank" rel="noreferrer">
                  {d.title}
                </a>{" "}
                <span className="muted">
                  {d.author}, {relative(d.created_at)}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  );
}

/**
 * The map explains what is selected (design S26). A part: what it is, what it receives and
 * sends, what depends on what, and what touches it now. A seam: its ends, owner, guard, what it
 * carries, its flows, and the O-rows open plans have written for it.
 */
export function Inspector({ overview, selection, onSelect, onSelectIssue }: Props) {
  const x = explain(overview, selection);
  if (!x) {
    return (
      <section className="inspector" data-testid="inspector" aria-label="Selected on the map">
        <p className="warn">
          {selection.id} is not in map {overview.map.version}.
        </p>
      </section>
    );
  }
  const close = (
    <button type="button" onClick={() => onSelect(null)}>
      Close
    </button>
  );
  if (x.kind === "seam") {
    const s = x.seam;
    return (
      <section className="inspector" data-testid="inspector" aria-label={`Seam ${s.id}`}>
        <header>
          <h2 className="mono">{s.id}</h2>
          <span className="kind">seam</span>
          {close}
        </header>
        <p className={s.description ? undefined : "gap"}>{s.description ?? `What this seam is for is ${NOT_DESCRIBED}.`}</p>
        <dl>
          <dt>Between</dt>
          <dd>
            <Ref target={{ kind: "node", id: s.from }} onSelect={onSelect} /> <span className="muted">depends on</span> <Ref target={{ kind: "node", id: s.to }} onSelect={onSelect} />
          </dd>
          <dt>Owner</dt>
          <dd>
            <Ref target={{ kind: "node", id: s.owner }} onSelect={onSelect} /> <span className="muted">is woken when it breaks</span>
          </dd>
          <dt>Carries</dt>
          <dd className={s.carries ? undefined : "gap"}>{s.carries ?? NOT_DESCRIBED}</dd>
          <dt>Guard</dt>
          <dd className={s.guard ? undefined : "gap"}>{s.guard ?? `no guard ${NOT_DESCRIBED.replace("not ", "")}`}</dd>
        </dl>
        <h3>Flows</h3>
        {x.flows.length ? <SeamFlows flows={x.flows} onSelect={onSelect} /> : <p className="gap">What crosses this seam is {NOT_DESCRIBED}.</p>}
        <h3>If it breaks at 3 am</h3>
        {x.oRows.length ? (
          <ul className="plain">
            {x.oRows.map(({ issue, row }, i) => (
              <li key={i}>
                <button type="button" className="link" onClick={() => onSelectIssue(issue)}>
                  {issueKey(issue)}
                </button>
                : {row.signal} <span className="muted">on panel</span> {row.panel}, <span className="muted">wakes</span> {row.owner}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">No open plan has written an O-row for this seam.</p>
        )}
        <h3>Touching it now</h3>
        <TouchingNow touching={x.touching} onSelectIssue={onSelectIssue} />
      </section>
    );
  }
  const n = x.node;
  return (
    <section className="inspector" data-testid="inspector" aria-label={`Part ${n.id}`}>
      <header>
        <h2 className="mono">{n.id}</h2>
        <span className="kind">{n.kind}</span>
        {close}
      </header>
      <p className={n.description ? undefined : "gap"}>{n.description ?? `What this part is for is ${NOT_DESCRIBED}.`}</p>
      {x.codeUrl && (
        <p>
          <a href={x.codeUrl} target="_blank" rel="noreferrer">
            {n.repo}
            {n.path ? ` · ${n.path}` : ""}
          </a>
        </p>
      )}
      <h3>Receives</h3>
      <FlowList lines={x.incoming} direction="from" onSelect={onSelect} />
      <h3>Sends</h3>
      <FlowList lines={x.outgoing} direction="to" onSelect={onSelect} />
      {x.seamsWithoutFlows.length > 0 && (
        <p className="gap">
          What crosses{" "}
          {x.seamsWithoutFlows.map((id, i) => (
            <span key={id}>
              {i > 0 && ", "}
              <Ref target={{ kind: "seam", id }} onSelect={onSelect} />
            </span>
          ))}{" "}
          is {NOT_DESCRIBED}.
        </p>
      )}
      <h3>Depends on</h3>
      {x.dependsOn.length ? (
        <ul className="plain">
          {x.dependsOn.map((d) => (
            <li key={d.seam}>
              <Ref target={{ kind: "node", id: d.part }} onSelect={onSelect} /> <span className="muted">over</span> <Ref target={{ kind: "seam", id: d.seam }} onSelect={onSelect} />
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">Nothing on the map.</p>
      )}
      <h3>Depended on by</h3>
      {x.dependants.length ? (
        <ul className="plain">
          {x.dependants.map((d) => (
            <li key={d.seam}>
              <Ref target={{ kind: "node", id: d.part }} onSelect={onSelect} /> <span className="muted">over</span> <Ref target={{ kind: "seam", id: d.seam }} onSelect={onSelect} />
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">Nothing on the map.</p>
      )}
      <h3>Touching it now</h3>
      <TouchingNow touching={x.touching} onSelectIssue={onSelectIssue} />
    </section>
  );
}
