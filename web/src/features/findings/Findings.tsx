import type { Finding } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";

/** A report window in the largest whole unit: "1 h", "30 min", "90 s", "1 d". */
export function windowText(secs: number): string {
  for (const [unit, n] of [["d", 86_400], ["h", 3600], ["min", 60]] as const) {
    if (secs % n === 0) {
      return `${secs / n} ${unit}`;
    }
  }
  return `${secs} s`;
}

export function findingText(f: Finding): string {
  switch (f.kind) {
    case "flow_off_seam":
      return `${issueKey(f.issue)} plans a flow from ${f.from} to ${f.to} over ${f.seam}, which does not join them`;
    case "unknown_ids":
      return `${issueKey(f.issue)} names ids the map does not have: ${f.ids.join(", ")}`;
    case "draft_owner_conflict":
      return `${issueKey(f.issue)}: ${f.other}'s session wrote a plan on ${f.branch}, whose draft ${f.owner} owns; it was not stored (taking a draft over is Import)`;
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
    case "comment_approval_ignored":
      return `${issueKey(f.issue)}: ${f.author}'s Approved: comment was posted after the cutover, so it does not count; plans are approved in Rubato`;
    case "unattributed_session":
      return `a session${f.repo ? ` in ${f.repo}` : ""} on ${f.branch ? `branch ${f.branch}` : "no branch"} is not tied to an issue; name its branch issue/<n>-…`;
    case "seam_quiet":
      return `no calls observed on ${f.seam} in the last ${windowText(f.window_secs)}, though its two parts both show traffic`;
    case "undeclared_traffic":
      return `${f.from} calls ${f.to} (${f.calls} call${f.calls === 1 ? "" : "s"} in the last window), but no seam on the map joins them`;
    case "observed_only_part": {
      const s = f.service.namespace ? `${f.service.namespace}/${f.service.name}` : f.service.name;
      return `${s} is running (${f.calls} call${f.calls === 1 ? "" : "s"} in the last window), but no part on the map says it is this service`;
    }
    case "plan_path_conflict":
      return `the maps of ${f.workspaces.join(" and ")} give ${f.repo} different plan paths; the first one is used`;
    case "id_collision":
      return `${f.id} names different parts in ${f.workspaces.join(" and ")}; plans that name it may mean either`;
    case "approval_record_mismatch":
      return `${issueKey(f.issue)}: the plan on ${f.branch} carries an approval record Rubato did not make (${f.reason}); no link comment is posted`;
    case "link_comment_waiting":
      return `${issueKey(f.issue)}: the link comment for the handoff has not been found on the issue for 30 minutes; it may be posted again`;
    case "link_waiting_for_approver":
      return `${issueKey(f.issue)}: the link comment waits for ${f.approver} to log in to Rubato again`;
    case "adrs_not_accepted":
      return `${issueKey(f.issue)}: the approval covers ${f.adrs.join(", ")}, which live in the map repo and stay proposed until accepted there`;
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
