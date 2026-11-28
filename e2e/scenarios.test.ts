// pnpm --filter @rubato/e2e test: the scenario check and the coverage report (issue #198, ADR 0069).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { type Coverage, coverage, fromPlaywright, fromRun, goneSince, idsIn, readFeatures, renderMarkdown, runChecksNotRun, validate } from "./scenarios.ts";
import type { RunResults } from "./run-checks.ts";

const here = dirname(fileURLToPath(import.meta.url));
const fx = (p: string) => join(here, "fixtures/scenarios", p);
const json = <T>(p: string): T => JSON.parse(readFileSync(fx(p), "utf8")) as T;
const cli = (...args: string[]) =>
  spawnSync(process.execPath, ["--experimental-strip-types", join(here, "scenarios.ts"), ...args], { encoding: "utf8", env: { ...process.env, GITHUB_ACTIONS: "", GITHUB_STEP_SUMMARY: "" } });

test("the reader finds features, scenarios, ids and steps; tags of a Rule or Background are not a scenario's", () => {
  const { features, problems } = readFeatures(fx("good"));
  assert.deepEqual(problems, []);
  assert.deepEqual(
    features.map((f) => [f.capability, f.name]),
    [
      ["auth", "Logging in"],
      ["map", "The map"],
    ],
  );
  const auth = features[0]!.scenarios;
  assert.deepEqual(
    auth.map((s) => s.id),
    ["@auth.all-pass", "@auth.one-failed", "@auth.nobody", "@auth.selftest-only", "@auth.never-ran"],
  );
  assert.equal(auth[0]!.steps, 2);
  assert.equal(auth[0]!.line, 6);
  const map = features[1]!.scenarios[0]!;
  assert.equal(map.id, "@map.skipped");
  assert.deepEqual(map.tags, ["@map.skipped", "@slow"]);
});

test("must fail: a scenario without an id, with two, of another capability, malformed, twice, or with no steps", () => {
  const { problems } = readFeatures(fx("bad"));
  const has = (re: RegExp) => assert.ok(problems.some((p) => re.test(p)), `${re} in ${JSON.stringify(problems, null, 1)}`);
  has(/auth\.feature:3: scenario "No id tag" has no id tag @auth\.<slug>/);
  has(/"Two id tags" has 2 id tags/);
  has(/@map\.wrong-file is in auth\.feature; its id starts @auth\./);
  has(/@auth\.Not_Lowercase is not an id/);
  has(/@auth\.twice is already the id of "First of a duplicate"/);
  has(/"No steps" has no steps/);
  assert.equal(problems.length, 6);
});

test("Playwright's JSON puts the @ back on tags, walks nested suites, and reads each status", () => {
  const tests = fromPlaywright(json("playwright.json"));
  assert.deepEqual(
    tests.map((t) => [t.title, t.status, t.tags.join(" ")]),
    [
      ["passes", "passed", "@auth.all-pass @auth.one-failed"],
      ["fails", "failed", "@auth.one-failed"],
      ["must fail: the check catches it", "passed", "@auth.selftest-only @selftest"],
      ["writes a screenshot", "skipped", "@tooling"],
      ["is skipped", "skipped", "@map.skipped"],
      ["was never run", "not run", "@auth.never-ran"],
    ],
  );
  assert.equal(tests[0]!.file, "web/e2e/login.spec.ts");
  assert.equal(tests[0]!.project, "dev");
});

test("each scenario's state: covered, partly, not covered (selftests do not count), not run", () => {
  const { features } = readFeatures(fx("good"));
  const tests = [...fromPlaywright(json("playwright.json")), ...fromRun(json<RunResults>("run.json"))];
  const rows = coverage(features, tests);
  const state = (id: string) => rows.find((r) => r.scenario.id === id) as Coverage;
  assert.equal(state("@auth.all-pass").state, "covered");
  assert.equal(state("@auth.all-pass").tests.length, 2);
  assert.equal(state("@auth.one-failed").state, "partly");
  assert.deepEqual([state("@auth.one-failed").passed, state("@auth.one-failed").failed], [1, 1]);
  assert.equal(state("@auth.nobody").state, "not covered");
  assert.equal(state("@auth.selftest-only").state, "not covered");
  assert.equal(state("@auth.selftest-only").selftests.length, 1);
  assert.equal(state("@auth.never-ran").state, "not run");
  assert.equal(state("@map.skipped").state, "partly");
  assert.equal(state("@map.skipped").skipped, 1);
});

test("the Markdown report: totals, one row per scenario with its tests, the uncovered listed, selftests and tooling apart", () => {
  const { features, problems } = readFeatures(fx("good"));
  const tests = [...fromPlaywright(json("playwright.json")), ...fromRun(json<RunResults>("run.json"))];
  const v = validate(features, problems, tests);
  assert.ok(v.ok);
  const md = renderMarkdown(coverage(features, tests), v, tests, ["read web/e2e results from x.json"]);
  assert.match(md, /Checked 6 scenarios and 7 tests/);
  assert.match(md, /✅ 1 covered and passing · 🟡 2 partly covered · ⏸ 1 not run · ⬜ 2 not covered/);
  assert.match(md, /\| `@auth\.one-failed` One citing test failed \| 🟡 1 of 2 passed; 1 failed \|/);
  assert.match(md, /\| `@map\.skipped` A citing test was skipped \| 🟡 0 of 1 passed; 1 skipped \|/);
  assert.match(md, /\| `@auth\.never-ran` Its tests did not run \| ⏸ not run \(1\) \|/);
  assert.match(md, /passes \(`web\/e2e\/login\.spec\.ts:3 \[dev\]`, passed\)/);
  assert.match(md, /### Not covered \(2\)\n\n- `@auth\.nobody`/);
  assert.match(md, /### Selftests, covering no scenario \(1\)/);
  assert.match(md, /### Tooling, covering no scenario \(1\)/);
  assert.match(md, /read web\/e2e results from x\.json/);
});

test("the check passes when every test cites a known scenario, and an uncovered scenario only warns", () => {
  const r = cli("check", "--features", fx("good"), "--playwright", fx("covering.json"), "--run", fx("run.json"));
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /warning: @auth\.nobody "No test cites it"/);
  assert.match(r.stdout, /checked 6 scenarios and 2 tests: ok, 5 not covered/);
  const line = JSON.parse(r.stdout.split("\n").find((l) => l.startsWith("{")) as string);
  assert.equal(line.event, "rubato.scenarios.check");
  assert.equal(line.outcome, "pass");
});

test("must fail: a test citing a scenario no feature has fails the check", () => {
  const r = cli("check", "--features", fx("good"), "--playwright", fx("unknown.json"), "--run", fx("run.json"));
  assert.equal(r.status, 1, r.stdout);
  assert.match(r.stdout, /error: web\/e2e\/x\.spec\.ts:1 \[dev\] "cites a scenario nobody wrote" cites @auth\.no-such-scenario, which no scenario in features\/ has/);
  assert.match(r.stdout, /"unknown":1/);
  assert.match(r.stdout, /"outcome":"fail"/);
});

test("must fail: a test citing no scenario (a selftest tag alone included) fails the check", () => {
  const r = cli("check", "--features", fx("good"), "--playwright", fx("untagged.json"), "--run", fx("run.json"));
  assert.equal(r.status, 1, r.stdout);
  assert.match(r.stdout, /"cites nothing" cites no scenario/);
  assert.match(r.stdout, /"must fail: cites only selftest" cites no scenario/);
  assert.match(r.stdout, /"untagged":2/);
});

test("must fail: a malformed feature file fails the check", () => {
  const r = cli("check", "--features", fx("bad"), "--playwright", fx("covering.json"), "--run", fx("run.json"));
  assert.equal(r.status, 1, r.stdout);
  assert.match(r.stdout, /"invalid":6/);
});

test("every id the base had is still there: a reworded scenario keeps its id", () => {
  const { features } = readFeatures(fx("good"));
  const now = features.flatMap((f) => f.scenarios.flatMap((s) => (s.id ? [s.id] : [])));
  // The base's files, as text: the same ids under other names, and tags that are not ids.
  const base = idsIn(now.map((id) => `Feature: x\n\n  @wip ${id}\n  Scenario: worded differently back then\n    Given a thing\n`));
  assert.equal(base.length, now.length);
  assert.ok(base.length > 0);
  assert.deepEqual(goneSince(base, features), []);
});

test("must fail: an id the base had and no scenario has now is reported", () => {
  const { features } = readFeatures(fx("good"));
  const base = idsIn(["Feature: x\n\n  @pins.renamed-away\n  Scenario: was here\n    Given a thing\n"]);
  assert.deepEqual(goneSince(base, features), ["@pins.renamed-away"]);
});

test("--base reads features/ at a git ref; a ref that cannot be read is bad input", () => {
  const ok = cli("check", "--playwright", fx("covering.json"), "--run", fx("run.json"), "--features", join(here, "../features"), "--base", "HEAD");
  assert.match(ok.stdout, /ids of HEAD checked, \d+ gone/);
  const bad = cli("check", "--playwright", fx("covering.json"), "--run", fx("run.json"), "--features", fx("good"), "--base", "no-such-ref-357");
  assert.equal(bad.status, 2);
  assert.match(bad.stderr, /no-such-ref-357/);
});

test("report writes the file and the job summary, and exits 0 although a test failed", () => {
  const dir = mkdtempSync(join(tmpdir(), "scenarios-"));
  const summary = join(dir, "summary.md");
  writeFileSync(summary, "");
  const r = spawnSync(
    process.execPath,
    ["--experimental-strip-types", join(here, "scenarios.ts"), "report", "--features", fx("good"), "--playwright", fx("playwright.json"), "--run", fx("run.json"), "--out", join(dir, "out.md")],
    { encoding: "utf8", env: { ...process.env, GITHUB_ACTIONS: "true", GITHUB_STEP_SUMMARY: summary } },
  );
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const md = readFileSync(join(dir, "out.md"), "utf8");
  assert.match(md, /## Scenario coverage/);
  assert.equal(readFileSync(summary, "utf8"), md);
  assert.match(r.stdout, /::warning file=.*auth\.feature,line=\d+,title=Scenario not covered::@auth\.nobody/);
});

test("without results, run.ts's checks come from run-checks.ts as not run", () => {
  const tests = runChecksNotRun();
  assert.ok(tests.length >= 10);
  assert.ok(tests.every((t) => t.status === "not run" && t.suite === "run" && t.tags.length > 0));
});
