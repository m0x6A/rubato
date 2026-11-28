import type { ConnectedRepo, GraphOverview, GraphSourceStatus, IssueRef, MapRef, Overview } from "../../protocol.ts";
import { graphLine, graphOf } from "./graph.ts";
import { RepoWork } from "./RepoWork.tsx";
import { type RepoFeatures, repoFeatures } from "./provides.ts";

type Max = { open_issues: number; plans: number; sessions: number; open_pulls: number };

/** The team's largest count of each kind, so every repo's bars share one scale. */
export function rackScale(repos: ConnectedRepo[]): Max {
  const top = (f: (r: ConnectedRepo) => number) => Math.max(1, ...repos.map(f));
  return { open_issues: top((r) => r.open_issues), plans: top((r) => r.plans), sessions: top((r) => r.sessions), open_pulls: top((r) => r.open_pulls ?? 0) };
}

/** One repo as a rack unit (issue #81, design §6): two lights (in the map, covered by the App) and four bars. */
export function RackUnit({ repo: r, max, provides, graph, regenSecs, now }: { repo: ConnectedRepo; max: Max; provides?: RepoFeatures | undefined; graph?: GraphSourceStatus | undefined; regenSecs?: number | undefined; now?: number | undefined }) {
  const bars: [keyof Max, number, string][] = [
    ["open_issues", r.open_issues, "open issues"],
    ["plans", r.plans, r.plans === 1 ? "plan" : "plans"],
    ["sessions", r.sessions, r.sessions === 1 ? "session" : "sessions"],
    ["open_pulls", r.open_pulls ?? 0, "open PRs"],
  ];
  return (
    <div className="rack" data-testid={`repo-${r.repo}`}>
      <div className="rack-head">
        <span className="rack-name">{r.repo}</span>
        <span className={r.in_map ? "rack-led on" : "rack-led off"}>in map: {r.in_map ? "yes" : <span className="warn">no</span>}</span>
        <span className={r.covered ? "rack-led on" : "rack-led off"}>app: {r.covered ? "covered" : <span className="warn">not covered</span>}</span>
      </div>
      <div className="rack-bars">
        {bars.map(([key, n, label]) => (
          <span key={key} className={`rack-bar ${key}`}>
            <i aria-hidden="true">
              <b data-share={(n / max[key]).toFixed(2)} style={{ height: `${Math.round((n / max[key]) * 100)}%` }} />
            </i>
            <em>{n}</em> {label}
          </span>
        ))}
      </div>
      {r.unattributed > 0 && <span className="badge approval-stale">{r.unattributed} not tied to an issue</span>}
      {graph && <GraphLine name={r.repo} source={graph} regenSecs={regenSecs ?? 900} now={now} />}
      {provides && <Provides repo={r.repo} f={provides} />}
    </div>
  );
}

/** How one context graph source stands (ADR 0084): its head, when a tick last found it current,
 * its size; or that it is failing, stale or not built yet. Said in words, not only in colour. */
export function GraphLine({ name, source, regenSecs, now, label = "context graph" }: { name: string; source: GraphSourceStatus; regenSecs: number; now?: number | undefined; label?: string }) {
  const line = graphLine(source, regenSecs, now);
  return (
    <p className={`graph-line ${line.state}`} data-testid={`graph-${name}`} data-state={line.state}>
      {label}: {line.text}
    </p>
  );
}

/** The sources that belong to no one repository: this workspace's map and pins, and the team's
 * shared facts. Shown once, above the repositories. Nothing with the flag off. */
export function SharedGraphLines({ graph, workspace, now }: { graph: GraphOverview | null | undefined; workspace: string; now?: number | undefined }) {
  const sources = graph?.sources ?? [];
  // The server lists only this tenant's sources: its workspaces as `workspace/<tenant>/<id>` and
  // its shared facts as `team/<tenant>` (ADR 0087).
  const ws = sources.find((s) => s.kind === "workspace" && s.source.endsWith(`/${workspace}`));
  const team = sources.find((s) => s.kind === "team");
  if (!graph || (!ws && !team)) {
    return null;
  }
  return (
    <div className="graph-shared" data-testid="graph-shared">
      {ws && <GraphLine name={ws.source} source={ws} regenSecs={graph.regen_secs} now={now} label="context graph, this workspace's map and pins" />}
      {team && <GraphLine name="team" source={team} regenSecs={graph.regen_secs} now={now} label="context graph, the team's issues, plans and pull requests" />}
    </div>
  );
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** What the repo provides to the whole application, from the map (issue #89): folded to one line of counts. */
function Provides({ repo, f }: { repo: string; f: RepoFeatures }) {
  if (f.parts.length === 0) {
    return (
      <p className="muted provides" data-testid={`provides-${repo}`}>
        No part of the map lives in this repo.
      </p>
    );
  }
  return (
    <details className="provides" data-testid={`provides-${repo}`}>
      <summary>
        {plural(f.parts.length, "part", "parts")} · owns {plural(f.owns.length, "seam", "seams")} · uses {f.uses.length} · in {plural(f.scenarios.length, "journey", "journeys")}
      </summary>
      <ul className="plain">
        {f.parts.map((p) => (
          <li key={p.id}>
            <code>{p.id}</code>
            {p.description ? ` — ${p.description}` : ""}
          </li>
        ))}
      </ul>
      {f.owns.length > 0 && (
        <p>
          Owns {f.owns.map((s, i) => (
            <span key={s.id}>
              {i > 0 && ", "}
              <code>{s.id}</code>
            </span>
          ))}
        </p>
      )}
      {f.uses.length > 0 && (
        <p>
          Uses {f.uses.map((s, i) => (
            <span key={s.id}>
              {i > 0 && ", "}
              <code>{s.id}</code>
            </span>
          ))}
        </p>
      )}
      {f.scenarios.length > 0 && <p>In {f.scenarios.map((s) => s.name).join(", ")}</p>}
    </details>
  );
}

/** Connected repos (issue #80): each repo the map names, whether the App covers it, and its
 * open issues, plans and sessions; and the sessions Rubato cannot tie to an issue. */
export function ConnectedRepos({ overview, onOpenIssue, onPin }: { overview: Overview; onOpenIssue?: ((i: IssueRef) => void) | undefined; onPin?: ((i: IssueRef, t: MapRef) => void) | undefined }) {
  const repos = overview.repos ?? [];
  const max = rackScale(repos);
  const loose = (overview.unpinned_sessions ?? []).filter((s) => s.status !== "ended");
  return (
    <section data-testid="connected-repos">
      <h3>Connected repos</h3>
      <SharedGraphLines graph={overview.graph} workspace={overview.workspace ?? "default"} />
      <ul className="plain racks">
        {repos.map((r) => (
          <li key={r.repo}>
            <RackUnit repo={r} max={max} provides={repoFeatures(overview.map.map, r.repo)} graph={graphOf(overview.graph, r.repo)} regenSecs={overview.graph?.regen_secs} />
            {r.in_map && <RepoWork overview={overview} repo={r.repo} onOpenIssue={onOpenIssue} onPin={onPin} />}
          </li>
        ))}
        {repos.length === 0 && <li className="muted">The map names no repos.</li>}
      </ul>
      {loose.length > 0 && (
        <>
          <h4>Sessions not tied to an issue</h4>
          <ul className="plain" data-testid="unattributed">
            {loose.map((s, i) => (
              <li key={s.session_id ?? i}>
                <code>{s.repo ?? "no repo"}</code> on <code>{s.branch ?? "no branch"}</code> <span className="muted">· {s.tmux?.session ?? s.cwd}</span>
                <div className="muted">Name the branch issue/&lt;n&gt;-… and it shows on that issue.</div>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
