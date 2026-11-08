#!/usr/bin/env -S node --experimental-strip-types
/**
 * Structural check of the observability files: every alert in the "never silent" list exists,
 * every rule has a unit test that expects it to fire, every alert names a panel that exists on
 * the dashboard. Reports what it checked. `promtool test rules` is the semantic check.
 *
 *   check.ts [--rules <file>] [--tests <file>] [--dashboard <file>]
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const here = dirname(fileURLToPath(import.meta.url));
const REQUIRED = ["CompanionSilent", "SessionWorkingNoHooks", "PlanUnknownNodes", "SeamCollision", "GitHubRateLimitLow"];

function opt(argv: string[], name: string, fallback: string): string {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? (argv[i + 1] as string) : fallback;
}

export function check(rulesText: string, testsText: string, dashboardText: string): { problems: string[]; rules: number; tests: number; panels: number } {
  const problems: string[] = [];
  const rules = parseYaml(rulesText) as { groups: { rules: { alert: string; annotations?: { panel?: string } }[] }[] };
  const tests = parseYaml(testsText) as { tests: { alert_rule_test: { alertname: string; exp_alerts: unknown[] }[] }[] };
  const dashboard = JSON.parse(dashboardText) as { panels: { title: string; panels?: { title: string }[] }[] };
  const panelTitles = new Set(dashboard.panels.flatMap((p) => [p.title, ...(p.panels ?? []).map((q) => q.title)]));
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
