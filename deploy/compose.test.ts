/**
 * The stack check passes on the committed files and fails on purpose on `fixtures/bad/`.
 * Run with `pnpm deploy:test` (node's test runner).
 */
import assert from "node:assert/strict";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { checkDir } from "./check.ts";

const here = dirname(fileURLToPath(import.meta.url));

test("the committed stack passes", () => {
  const r = checkDir(here);
  assert.deepEqual(r.problems, []);
  assert.equal(r.services, 2);
  console.log(`checked ${r.services} services against ${r.rules} rules`);
});

test("must fail: the bad fixture breaks every rule", () => {
  const r = checkDir(resolve(here, "fixtures/bad"));
  assert.equal(r.problems.length, r.rules, r.problems.join("\n"));
  assert.deepEqual(
    r.problems.map((p) => p.split(":")[0]),
    [
      "app must exist and be in no profile",
      "only app may run without a profile",
      "livekit must exist and be only in the voice profile",
      "livekit image must be pinned to an exact vX.Y.Z",
      "livekit LIVEKIT_KEYS must be \"${RUBATO_LIVEKIT_API_KEY}",
      "livekit.yaml must hold no keys",
      "speech.bicep must not read the account key (listKeys)",
      "speech.bicep must pin every model version",
      "speech.bicep must keep key auth on (disableLocalAuth",
    ],
  );
});
