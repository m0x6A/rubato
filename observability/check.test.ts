/**
 * The observability check passes on the committed files and fails on purpose on a broken copy.
 * Run with `pnpm observability:test` (node's test runner).
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { check } from "./check.ts";

const here = dirname(fileURLToPath(import.meta.url));
const rules = readFileSync(resolve(here, "alerts/rubato.rules.yml"), "utf8");
const tests = readFileSync(resolve(here, "alerts/tests/rubato.test.yml"), "utf8");
const dashboard = readFileSync(resolve(here, "dashboards/rubato-overview.json"), "utf8");

type Panel = { title: string; type?: string; targets?: { expr?: string }[] };
/** A copy of the committed dashboard with one change applied. */
function changed(edit: (panels: Panel[]) => Panel[]): string {
  const d = JSON.parse(dashboard) as { panels: Panel[] };
  return JSON.stringify({ ...d, panels: edit(d.panels) });
}

test("the committed rules, rule tests and dashboard pass", () => {
  const r = check(rules, tests, dashboard);
  assert.deepEqual(r.problems, []);
  console.log(`checked ${r.rules} alert rules, ${r.tests} rule tests, ${r.panels} dashboard panels`);
});

test("a dashboard without the Map age panel is rejected", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Map age")));
  assert.deepEqual(r.problems, ['the dashboard has no "Map age" panel that queries rubato_map_loaded_at']);
});

test("a dashboard without the Backlog age panel is rejected", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Backlog age")));
  assert.deepEqual(r.problems, ['the dashboard has no "Backlog age" panel that queries rubato_backlog_listed_at']);
});

test("a dashboard without the Map lines panel is rejected (#71)", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Map lines")));
  assert.deepEqual(r.problems, ['the dashboard has no "Map lines" panel that queries rubato_backlog_map_lines_total']);
});

test("a dashboard without the Issues created panel is rejected (#71)", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Issues created")));
  assert.deepEqual(r.problems, ['the dashboard has no "Issues created" panel that queries rubato_issues_created_total', 'the dashboard has no "Issues created" panel that queries rubato_issues_opening']);
});

test("a dashboard without the Map proposal age panel is rejected", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Map proposal age")));
  assert.deepEqual(r.problems, ['the dashboard has no "Map proposal age" panel that queries rubato_map_proposal_state_since']);
});

test("a Map proposal age panel on a metric with a unit suffix is rejected", () => {
  const withUnit = changed((panels) => panels.map((p) => (p.title === "Map proposal age" ? { ...p, targets: [{ expr: "time() - max by (rubato_outcome) (rubato_map_proposal_state_since_seconds > 0)" }] } : p)));
  assert.deepEqual(check(rules, tests, withUnit).problems, ['the dashboard has no "Map proposal age" panel that queries rubato_map_proposal_state_since']);
});

test("a dashboard without the Queue depth panel is rejected", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Queue depth")));
  assert.deepEqual(r.problems, ['the dashboard has no "Queue depth" panel that queries rubato_queue_depth']);
});

test("a dashboard without the Layout writes panel is rejected", () => {
  const r = check(rules, tests, changed((panels) => panels.filter((p) => p.title !== "Layout writes")));
  assert.deepEqual(r.problems, ['the dashboard has no "Layout writes" panel that queries rubato_layout_writes_total']);
});

test("a Backlog age panel on a metric with a unit suffix is rejected", () => {
  const withUnit = changed((panels) =>
    panels.map((p) => (p.title === "Backlog age" ? { ...p, targets: [{ expr: "time() - max by (rubato_repo) (rubato_backlog_listed_at_seconds)" }] } : p)),
  );
  assert.deepEqual(check(rules, tests, withUnit).problems, ['the dashboard has no "Backlog age" panel that queries rubato_backlog_listed_at']);
});

/** A copy of the committed dashboard with one panel's expressions replaced. */
const withExprs = (title: string, exprs: string[]) =>
  changed((panels) => panels.map((p) => (p.title === title && p.type !== "row" ? { ...p, targets: exprs.map((expr) => ({ expr })) } : p)));

test("a dashboard grouping by the bare label state is rejected", () => {
  const expr = "sum by (state) (rubato_issues_by_state)";
  const r = check(rules, tests, withExprs("Issues by state", [expr]));
  assert.deepEqual(r.problems, [`panel "Issues by state" groups by the bare label state: ${expr}`]);
});

test("a working-sessions line that sums over both binaries is rejected", () => {
  const sum = 'sum by (rubato_companion_id) (rubato_sessions_by_state{rubato_session_state="working"})';
  const hooks = "sum by (rubato_companion_id) (rate(rubato_hook_forwarded_total[1m]) * 60)";
  const problem = 'panel "Hooks/min vs working sessions" must take max by (rubato_companion_id) of rubato_sessions_by_state';
  assert.deepEqual(check(rules, tests, withExprs("Hooks/min vs working sessions", [hooks, sum])).problems, [problem]);
  assert.deepEqual(check(rules, tests, withExprs("Hooks/min vs working sessions", [hooks])).problems, [problem]);
});

test("a panel or rule that splits hooks forwarded by session is rejected", () => {
  // The counter carries no session id since #43; such a query would show one empty line.
  const bySession = "sum by (rubato_session_id) (rate(rubato_hook_forwarded_total[1m]) * 60)";
  assert.deepEqual(check(rules, tests, withExprs("Hooks/min vs working sessions", [bySession])).problems.filter((p) => p.includes("rubato_session_id")), [
    `panel "Hooks/min vs working sessions" queries rubato_hook_forwarded_total by rubato_session_id, which it does not carry: ${bySession}`,
  ]);
  const rule = rules.replace(
    "sum by (rubato_companion_id) (rate(rubato_hook_forwarded_total[2m])) == 0",
    'sum by (rubato_companion_id) (rate(rubato_hook_forwarded_total{rubato_session_id!=""}[2m])) == 0',
  );
  assert.notEqual(rule, rules, "the committed rule changed shape; update this test");
  assert.deepEqual(check(rule, tests, dashboard).problems, [
    'alert SessionWorkingNoHooks queries rubato_hook_forwarded_total by rubato_session_id, which it does not carry: sum by (rubato_companion_id) (rubato_sessions_by_state{rubato_session_state="working"}) > 0\nand\nsum by (rubato_companion_id) (rate(rubato_hook_forwarded_total{rubato_session_id!=""}[2m])) == 0\n',
  ]);
});

test("a Map age panel on a metric with a unit suffix is rejected", () => {
  const withUnit = changed((panels) =>
    panels.map((p) => (p.title === "Map age" ? { ...p, targets: [{ expr: "time() - max(rubato_map_loaded_at_seconds)" }] } : p)),
  );
  const r = check(rules, tests, withUnit);
  assert.deepEqual(r.problems, ['the dashboard has no "Map age" panel that queries rubato_map_loaded_at']);
});
