import type { Finding } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";

export function findingText(f: Finding): string {
  switch (f.kind) {
    case "flow_off_seam":
      return `${issueKey(f.issue)} plans a flow from ${f.from} to ${f.to} over ${f.seam}, which does not join them`;
    case "unknown_ids":
      return `${issueKey(f.issue)} names ids the map does not have: ${f.ids.join(", ")}`;
    case "seam_collision":
      return `two open plans touch ${f.seam}: ${f.issues.map(issueKey).join(", ")}`;
    case "plan_parse_error":
      return `${issueKey(f.issue)}: ${f.path} does not parse: ${f.error}`;
    case "map_version_mismatch":
      return `${issueKey(f.issue)} was launched with map ${f.launched_with}; the server holds ${f.current}`;
    case "approval_stale":
      return `${issueKey(f.issue)}: the plan changed after it was approved (approved ${f.approved_sha.slice(0, 7)}, now ${f.current_sha.slice(0, 7)})`;
    case "repo_not_covered":
      return `${f.repo} is not covered by the GitHub App installation; its branches, plans and pull requests cannot be read`;
    case "approved_without_fit":
      return `${issueKey(f.issue)} was approved, but its plan does not fit the map, so the approval does not count`;
    case "unattributed_session":
      return `a session${f.repo ? ` in ${f.repo}` : ""} on ${f.branch ? `branch ${f.branch}` : "no branch"} is not tied to an issue; name its branch issue/<n>-…`;
  }
}

export function Findings({ findings }: { findings: Finding[] }) {
  if (findings.length === 0) {
    return null;
  }
  return (
    <div className="findings" data-testid="findings">
      <strong>
        {findings.length} finding{findings.length === 1 ? "" : "s"}
      </strong>
      <ul>
        {findings.map((f, i) => (
          <li key={i}>{findingText(f)}</li>
        ))}
      </ul>
    </div>
  );
}
