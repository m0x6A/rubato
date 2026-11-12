/**
 * The promtool rule tests catch what they claim to: each mutant below breaks one alert rule the
 * way a careless edit would, and `promtool test rules` must fail on exactly the case named.
 * Run with `pnpm observability:rules`. Needs promtool (`PROMTOOL`, or `promtool` on PATH) and
 * fails without it: a check that skips itself is silent. CI runs it after installing promtool.
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const rulesPath = resolve(here, "alerts/rubato.rules.yml");
const testsPath = resolve(here, "alerts/tests/rubato.test.yml");
const promtool = process.env.PROMTOOL ?? "promtool";

type Mutant = {
  /** What the rule test case is there to prove. */
  why: string;
  from: string;
  to: string;
  /** The failure promtool must report: alert name and evaluation time of the catching case. */
  caughtBy: string;
};

const mutants: Mutant[] = [
  {
    why: "#26 S5: a working gauge back at 0 clears SessionWorkingNoHooks",
    from: 'sum by (rubato_companion_id) (rubato_sessions_by_state{rubato_session_state="working"}) > 0',
    to: 'sum by (rubato_companion_id) (rubato_sessions_by_state{rubato_session_state="working"})',
    caughtBy: "alertname: SessionWorkingNoHooks, time: 10m",
  },
];

/** Run the committed rule tests against `rules`, in a scratch copy of the alerts folder. */
function run(rules: string): { status: number | null; out: string; error?: Error } {
  const dir = mkdtempSync(join(tmpdir(), "rubato-rules-"));
  try {
    mkdirSync(join(dir, "tests"));
    writeFileSync(join(dir, "rubato.rules.yml"), rules);
    copyFileSync(testsPath, join(dir, "tests/rubato.test.yml"));
    const r = spawnSync(promtool, ["test", "rules", join(dir, "tests/rubato.test.yml")], { encoding: "utf8" });
    return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}`, error: r.error };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("each rule mutant is caught by the rule test case written for it", () => {
  const rules = readFileSync(rulesPath, "utf8");
  const baseline = run(rules);
  if (baseline.error) assert.fail(`promtool not found (${promtool}); set PROMTOOL: ${baseline.error.message}`);
  assert.equal(baseline.status, 0, `the committed rules pass their tests:\n${baseline.out}`);

  for (const m of mutants) {
    assert.equal(rules.split(m.from).length, 2, `mutant "${m.why}": the text to change appears once`);
    const r = run(rules.replace(m.from, m.to));
    assert.notEqual(r.status, 0, `mutant "${m.why}" passed the rule tests`);
    // promtool prints `FAILED:` once per file and one `alertname: …, time: …,` line per
    // failing evaluation, so the case lines are what to count.
    const failed = [...r.out.matchAll(/^\s+(alertname: \S+, time: \S+),/gm)].map((x) => x[1]);
    assert.deepEqual(failed, [m.caughtBy], `mutant "${m.why}" should fail its own case only:\n${r.out}`);
  }
  console.log(`promtool caught ${mutants.length}/${mutants.length} rule mutants`);
});
