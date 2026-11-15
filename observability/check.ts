#!/usr/bin/env -S node --experimental-strip-types
/**
 * Structural check of the observability files: every alert in the "never silent" list exists,
 * every rule has a unit test that expects it to fire, every alert names a panel that exists on
 * the dashboard, the "Map age" panel queries `rubato_map_loaded_at`, the "Interactive map" panels
 * of issue #48 exist and query their metrics, `CompanionSilent` and the "Companions" panel query
 * the threshold `rubato_companion_silent_secs` (#105), no expression groups by the
 * bare label `state`, the working-sessions line takes `max by`, and no two panels share an id
 * (Grafana keys panels by id, so a second one is dropped or overwritten). Reports what it checked.
 * `promtool test rules` is the semantic check; `check.test.ts` makes this one fail on purpose.
 *
 *   check.ts [--rules <file>] [--tests <file>] [--dashboard <file>]
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const here = dirname(fileURLToPath(import.meta.url));
const REQUIRED = [
  "CompanionSilent",
  "SessionWorkingNoHooks",
  "PlanUnknownNodes",
  "SeamCollision",
  "GitHubRateLimitLow",
  "ServerPollSilent",
  // Issue #122 (ADRs 0051, 0054, 0055).
  "SessionMessageStuck",
  "ApprovalNoHandoff",
  "HandoffNoLinkComment",
  "ApprovalRecordMismatch",
  // Issue #138 (ADR 0062).
  "HeadlessTurnsKilled",
  // Issue #208 (ADR 0074).
  "GitHubWebhooksSilent",
  "GitHubWebhooksStuck",
];

/** Panel title and the series it must query. Later PRs of issue #48 add theirs. */
const INTERACTIVE_MAP: [string, string][] = [
  ["Backlog age", "rubato_backlog_listed_at"],
  ["Backlog issues", "rubato_backlog_issues"],
  ["Web writes rejected", "rubato_web_rejected_total"],
  ["Repos not covered", "rubato_github_repos_uncovered"],
  ["Layout writes", "rubato_layout_writes_total"],
  ["Queue depth", "rubato_queue_depth"],
  ["Map proposals", "rubato_map_proposals_total"],
  ["Map proposal age", "rubato_map_proposal_state_since"],
  // Issue #71, PR 5 (ADR 0027): the O-row of seam server-plan-file names this panel.
  ["Approvals", "rubato_plans_approval_stale"],
  // Issue #71.
  ["Map lines", "rubato_backlog_map_lines_total"],
  ["Issues created", "rubato_issues_created_total"],
  ["Issues created", "rubato_issues_opening"],
  // Issue #104 (ADR 0039): the row "Map versus territory" the O-row of seam web-server names.
  ["Traffic report age", "rubato_traffic_report_age"],
  ["Quiet seams and undeclared edges", "rubato_traffic_seams_quiet"],
  ["Quiet seams and undeclared edges", "rubato_traffic_undeclared_edges"],
  ["Quiet seams and undeclared edges", "rubato_traffic_unmapped_services"],
  // Issue #140 (ADRs 0059, 0060): the O-row of seam web-server names these.
  ["Quiet seams and undeclared edges", "rubato_traffic_ghost_parts"],
  ["Quiet seams and undeclared edges", "rubato_traffic_dismissals"],
  ["Traffic fetches", "rubato_traffic_fetches_total"],
  // Issue #106 (ADR 0043): both O-rows name this panel.
  ["Blocked, by station left from", "rubato_issues_blocked_from"],
  // Issue #105: the Companions panel shows the N that CompanionSilent compares against.
  ["Companions", "rubato_companion_silent_secs"],
  // Issue #122: the row "Planning in Rubato" the O-rows of its seams name.
  ["Session messages by outcome", "rubato_session_messages_total"],
  ["Oldest message waiting for an idle session", "rubato_session_messages_oldest_idle_queued_seconds"],
  ["Companions by capability", "rubato_companions_by_capability"],
  ["Approvals waiting for handoff", "rubato_plans_handoff_overdue"],
  ["Link comments missing", "rubato_plans_link_comment_missing"],
  ["Approval records that differ", "rubato_plans_approval_record_mismatch"],
  ["Draft revisions", "rubato_drafts_revisions_total"],
  ["Draft revisions", "rubato_drafts_refused_total"],
  ["Session log", "rubato_session_log_entries_total"],
  ["Session log", "rubato_session_log_dropped_total"],
  // Issue #142 (ADR 0061): the O-rows of the skill seams and companion-claude-code name this panel.
  ["Skills emitting", "traces_service_graph_request_total"],
];

type Panel = { id?: number; title: string; type?: string; targets?: { expr?: string }[] };

function opt(argv: string[], name: string, fallback: string): string {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? (argv[i + 1] as string) : fallback;
}

export function check(rulesText: string, testsText: string, dashboardText: string): { problems: string[]; rules: number; tests: number; panels: number } {
  const problems: string[] = [];
  const rules = parseYaml(rulesText) as { groups: { rules: { alert: string; expr?: string; annotations?: { panel?: string } }[] }[] };
  const tests = parseYaml(testsText) as { tests: { alert_rule_test: { alertname: string; exp_alerts: unknown[] }[] }[] };
  const dashboard = JSON.parse(dashboardText) as { panels: (Panel & { panels?: Panel[] })[] };
  const panels: Panel[] = dashboard.panels.flatMap((p) => [p, ...(p.panels ?? [])]);
  const panelTitles = new Set(panels.map((p) => p.title));
  const byId = new Map<number, string[]>();
  for (const p of panels) {
    if (p.id === undefined) {
      problems.push(`panel ${JSON.stringify(p.title)} has no id`);
    } else {
      byId.set(p.id, [...(byId.get(p.id) ?? []), p.title]);
    }
  }
  for (const [id, titles] of byId) {
    if (titles.length > 1) {
      problems.push(`panel id ${id} is used by ${titles.length} panels: ${titles.map((t) => JSON.stringify(t)).join(", ")}`);
    }
  }
  const exprs = (title: string) => panels.filter((p) => p.title === title && p.type !== "row").flatMap((p) => (p.targets ?? []).map((t) => t.expr ?? ""));
  const alerts = rules.groups.flatMap((g) => g.rules);
  const names = new Set(alerts.map((a) => a.alert));
  for (const r of REQUIRED) {
    if (!names.has(r)) {
      problems.push(`required alert ${r} is missing from the rules`);
    }
  }
  const firing = new Set(tests.tests.flatMap((t) => t.alert_rule_test).filter((t) => t.exp_alerts.length > 0).map((t) => t.alertname));
  for (const a of alerts) {
    if (!firing.has(a.alert)) {
      problems.push(`alert ${a.alert} has no test that makes it fire`);
    }
    const panel = a.annotations?.panel;
    if (!panel) {
      problems.push(`alert ${a.alert} names no dashboard panel`);
    } else if (!panelTitles.has(panel)) {
      problems.push(`alert ${a.alert} names panel ${JSON.stringify(panel)} which is not on the dashboard`);
    }
  }
  // #105: CompanionSilent fires past the server's configured N, not a literal of its own.
  for (const a of alerts.filter((x) => x.alert === "CompanionSilent")) {
    if (!/\brubato_companion_silent_secs\b/.test(a.expr ?? "")) {
      problems.push(`alert CompanionSilent must compare against rubato_companion_silent_secs: ${a.expr}`);
    }
  }
  // The exact name: a unit on the gauge would export it as rubato_map_loaded_at_seconds.
  if (!exprs("Map age").some((e) => /\brubato_map_loaded_at\b/.test(e))) {
    problems.push(`the dashboard has no "Map age" panel that queries rubato_map_loaded_at`);
  }
  // Issue #48: each panel a 3 am row names, with the exact metric (no unit suffix on the gauge).
  for (const [title, metric] of INTERACTIVE_MAP) {
    if (!exprs(title).some((e) => new RegExp(`\\b${metric}\\b`).test(e))) {
      problems.push(`the dashboard has no ${JSON.stringify(title)} panel that queries ${metric}`);
    }
  }
  // Our labels are rubato_*: `rubato.issues.by_state` carries rubato_issue_state, not `state`.
  for (const p of panels) {
    for (const t of p.targets ?? []) {
      const grouped = [...(t.expr ?? "").matchAll(/\b(?:by|without)\s*\(([^)]*)\)/g)].flatMap((m) => (m[1] ?? "").split(",").map((l) => l.trim()));
      if (grouped.includes("state")) {
        problems.push(`panel ${JSON.stringify(p.title)} groups by the bare label state: ${t.expr}`);
      }
    }
  }
  // #43: rubato.hook.forwarded carries no session id (one series per session grew for the life
  // of the companion); the rubato.hook.forward span has it. A query that splits or filters the
  // counter by it shows nothing.
  const bySession = (e: string) => /\brubato_hook_forwarded_total\b/.test(e) && /\brubato_session_id\b/.test(e);
  for (const a of alerts) {
    if (bySession(a.expr ?? "")) {
      problems.push(`alert ${a.alert} queries rubato_hook_forwarded_total by rubato_session_id, which it does not carry: ${a.expr}`);
    }
  }
  for (const p of panels) {
    for (const t of p.targets ?? []) {
      if (bySession(t.expr ?? "")) {
        problems.push(`panel ${JSON.stringify(p.title)} queries rubato_hook_forwarded_total by rubato_session_id, which it does not carry: ${t.expr}`);
      }
    }
  }
  // rubato.sessions.by_state is the server's alone since #43 (from each companion snapshot). The
  // panel keeps max by so it stays right while a companion built before #43, which exported its
  // own copy with the same labels, is still running.
  const working = exprs("Hooks/min vs working sessions").filter((e) => e.includes("rubato_sessions_by_state"));
  if (working.length === 0 || !working.every((e) => /^max by \(rubato_companion_id\) \(rubato_sessions_by_state\b/.test(e))) {
    problems.push(`panel "Hooks/min vs working sessions" must take max by (rubato_companion_id) of rubato_sessions_by_state`);
  }
  return { problems, rules: alerts.length, tests: tests.tests.flatMap((t) => t.alert_rule_test).length, panels: panelTitles.size };
}

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  const argv = process.argv.slice(2);
  const rules = readFileSync(opt(argv, "--rules", resolve(here, "alerts/rubato.rules.yml")), "utf8");
  const tests = readFileSync(opt(argv, "--tests", resolve(here, "alerts/tests/rubato.test.yml")), "utf8");
  const dashboard = readFileSync(opt(argv, "--dashboard", resolve(here, "dashboards/rubato-overview.json")), "utf8");
  const r = check(rules, tests, dashboard);
  console.log(`observability check: ${r.rules} alert rules, ${r.tests} rule tests, ${r.panels} dashboard panels; ${r.problems.length} problem(s)`);
  for (const p of r.problems) {
    console.error(`  - ${p}`);
  }
  process.exitCode = r.problems.length === 0 ? 0 : 1;
}
