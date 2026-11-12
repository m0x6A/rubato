import type { Overview } from "../../protocol.ts";

/** Connected repos (issue #80): each repo the map names, whether the App covers it, and its
 * open issues, plans and sessions; and the sessions Rubato cannot tie to an issue. */
export function ConnectedRepos({ overview }: { overview: Overview }) {
  const repos = overview.repos ?? [];
  const loose = (overview.unpinned_sessions ?? []).filter((s) => s.status !== "ended");
  return (
    <section data-testid="connected-repos">
      <h3>Connected repos</h3>
      <table className="mini">
        <thead>
          <tr>
            <th>Repo</th>
            <th>In map</th>
            <th>App</th>
            <th>Open issues</th>
            <th>Plans</th>
            <th>Sessions</th>
            <th>Not tied</th>
          </tr>
        </thead>
        <tbody>
          {repos.map((r) => (
            <tr key={r.repo} data-testid={`repo-${r.repo}`}>
              <td>{r.repo}</td>
              <td>{r.in_map ? "yes" : <span className="warn">no</span>}</td>
              <td>{r.covered ? "covered" : <span className="warn">not covered</span>}</td>
              <td>{r.open_issues}</td>
              <td>{r.plans}</td>
              <td>{r.sessions}</td>
              <td>{r.unattributed > 0 ? <span className="warn">{r.unattributed}</span> : 0}</td>
            </tr>
          ))}
          {repos.length === 0 && (
            <tr>
              <td colSpan={7} className="muted">
                The map names no repos.
              </td>
            </tr>
          )}
        </tbody>
      </table>
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
