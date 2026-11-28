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
  assert.equal(r.services, 3);
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
      "caddy must exist and be only in the public profile",
      "caddy image must be pinned to an exact X.Y.Z",
      "app must publish its port only as ${RUBATO_APP_BIND",
      "the edge network must have a fixed subnet and hold caddy and app, so RUBATO_TRUSTED_PROXY can name it",
      "Caddyfile must serve {$RUBATO_DOMAIN} and proxy to app",
      "Caddyfile must keep public certificates and the https redirect, serve no http",
      "foundry.bicep must not read the account key (listKeys)",
      "foundry.bicep must pin every model version",
      "foundry.bicep must keep key auth on (disableLocalAuth",
    ],
  );
});
