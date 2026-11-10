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

test("a Map age panel on a metric with a unit suffix is rejected", () => {
  const withUnit = changed((panels) =>
    panels.map((p) => (p.title === "Map age" ? { ...p, targets: [{ expr: "time() - max(rubato_map_loaded_at_seconds)" }] } : p)),
  );
  const r = check(rules, tests, withUnit);
  assert.deepEqual(r.problems, ['the dashboard has no "Map age" panel that queries rubato_map_loaded_at']);
});
