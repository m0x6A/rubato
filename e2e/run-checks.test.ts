// pnpm --filter @rubato/e2e test: the results e2e/run.ts writes (issue #198).
import assert from "node:assert/strict";
import { test } from "node:test";
import { CheckLog, RUN_CHECKS } from "./run-checks.ts";

const all = RUN_CHECKS.map((c) => c.key);

test("a run that reached and examined every check with no problem passes them all", () => {
  const log = new CheckLog();
  for (const k of all) {
    log.reach(k, 3);
  }
  const r = log.results();
  assert.equal(r.suite, "run");
  assert.equal(r.tests.length, RUN_CHECKS.length);
  assert.ok(r.tests.every((t) => t.status === "passed" && t.checked === 3 && t.problems.length === 0 && t.scenarios.length > 0));
});

test("must fail: a problem fails its own check and no other", () => {
  const log = new CheckLog();
  for (const k of all) {
    log.reach(k, 1);
  }
  log.problem("collision", "expected a seam collision");
  const r = log.results();
  assert.deepEqual(
    r.tests.filter((t) => t.status === "failed").map((t) => [t.key, t.problems]),
    [["collision", ["expected a seam collision"]]],
  );
  assert.deepEqual(log.all(), ["expected a seam collision"]);
});

test("must fail: a check reached with nothing examined fails", () => {
  const log = new CheckLog();
  for (const k of all) {
    log.reach(k, k === "hooks" ? 0 : 1);
  }
  const hooks = log.results().tests.find((t) => t.key === "hooks")!;
  assert.equal(hooks.status, "failed");
  assert.match(hooks.problems[0]!, /examined nothing/);
});

test("must fail: a run that stopped early fails the checks it did not reach, with the reason", () => {
  const log = new CheckLog();
  log.reach("pairing", 1);
  const r = log.results("timed out waiting for all three issues Designed");
  assert.equal(r.tests.find((t) => t.key === "pairing")!.status, "passed");
  const progress = r.tests.find((t) => t.key === "progress")!;
  assert.equal(progress.status, "failed");
  assert.deepEqual(progress.problems, ["the run stopped before this check: timed out waiting for all three issues Designed"]);
});
