import type { FitReason, IssueSummary, Overview } from "../../protocol.ts";

/** Plans Rubato can read, in two groups (issue #80): *proposed* (a plan, no counted approval)
 * and *open* (a counted approval, not merged). A merged plan is neither. */
export function groupPlans(ov: Overview): { proposed: IssueSummary[]; open: IssueSummary[] } {
  const withPlan = ov.issues.filter((i) => (i.plan || i.fit) && !i.pull_request?.merged_at && i.progress.state !== "done");
  const counted = (i: IssueSummary) => i.approval?.version === "current";
  const byNumber = (a: IssueSummary, b: IssueSummary) => a.issue.repo.localeCompare(b.issue.repo) || a.issue.number - b.issue.number;
  return {
    proposed: withPlan.filter((i) => !counted(i)).sort(byNumber),
    open: withPlan.filter(counted).sort(byNumber),
  };
}

/** One line per reason a plan does not fit the map (ADR 0029). */
export function fitReasonText(r: FitReason): string {
  switch (r.kind) {
    case "parse_error":
      return `the plan file does not parse: ${r.message}`;
    case "unknown_ids":
      return `names ids the map does not have: ${r.ids.join(", ")}`;
    case "flow_off_seam":
      return `a flow from ${r.from} to ${r.to} over ${r.seam}, which does not join them`;
    case "missing_o_row":
      return `seam ${r.seam} has no O-row (the 3 am question)`;
    case "model_off_part":
      return `data model ${r.model} sits on ${r.part}, which Affected parts does not name`;
    case "model_off_seam":
      return `data model ${r.model} crosses ${r.seam}, which no affected part touches`;
  }
}

/** The data models of a plan, grouped by the part that owns them, in Affected parts order. */
export function modelsByPart(issue: IssueSummary) {
  const plan = issue.plan;
  if (!plan) {
    return [];
  }
  const parts = [...new Set([...plan.affected_parts.map((p) => p.node), ...(plan.data_models ?? []).map((m) => m.part)])];
  return parts
    .map((part) => ({ part, models: (plan.data_models ?? []).filter((m) => m.part === part) }))
    .filter((g) => g.models.length > 0);
}
