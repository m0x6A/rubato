import type { ConnectedRepo, Overview } from "../../protocol.ts";

type Max = { open_issues: number; plans: number; sessions: number; open_pulls: number };

/** The team's largest count of each kind, so every repo's bars share one scale. */
export function rackScale(repos: ConnectedRepo[]): Max {
  const top = (f: (r: ConnectedRepo) => number) => Math.max(1, ...repos.map(f));
  return { open_issues: top((r) => r.open_issues), plans: top((r) => r.plans), sessions: top((r) => r.sessions), open_pulls: top((r) => r.open_pulls ?? 0) };
}

/** One repo as a rack unit (issue #81, design §6): two lights (in the map, covered by the App) and four bars. */
export function RackUnit({ repo: r, max }: { repo: ConnectedRepo; max: Max }) {
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
    </div>
  );
}

/** Connected repos (issue #80): each repo the map names, whether the App covers it, and its
 * open issues, plans and sessions; and the sessions Rubato cannot tie to an issue. */
export function ConnectedRepos({ overview }: { overview: Overview }) {
  const repos = overview.repos ?? [];
  const max = rackScale(repos);
  const loose = (overview.unpinned_sessions ?? []).filter((s) => s.status !== "ended");
  return (
    <section data-testid="connected-repos">
      <h3>Connected repos</h3>
      <ul className="plain racks">
        {repos.map((r) => (
          <li key={r.repo}>
            <RackUnit repo={r} max={max} />
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
