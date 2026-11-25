/**
 * Checks Rubato's stack files (#315): only `app` runs without a profile, LiveKit runs only under
 * the `voice` profile at a pinned version, and its keys come from the names Rubato signs with,
 * never from its config file. Run with `pnpm deploy:check`; tested by `compose.test.ts`.
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "yaml";

type Service = { profiles?: string[]; image?: string; environment?: Record<string, string> | string[] };
type Compose = { services?: Record<string, Service> };

/** What Rubato signs join tokens with (ADR 0073), as LiveKit's `key: secret` line. */
export const LIVEKIT_KEYS = "${RUBATO_LIVEKIT_API_KEY}: ${RUBATO_LIVEKIT_API_SECRET}";
const PINNED = /^livekit\/livekit-server:v\d+\.\d+\.\d+$/;

export type Result = { services: number; rules: number; problems: string[] };

export function check(composeText: string, livekitText: string): Result {
  const compose = (parse(composeText) ?? {}) as Compose;
  const livekit = (parse(livekitText) ?? {}) as Record<string, unknown>;
  const services = compose.services ?? {};
  const problems: string[] = [];
  let rules = 0;
  const rule = (ok: boolean, problem: string) => {
    rules += 1;
    if (!ok) problems.push(problem);
  };

  const app = services.app;
  rule(app !== undefined && (app.profiles ?? []).length === 0, "app must exist and be in no profile");

  const unprofiled = Object.entries(services)
    .filter(([name, s]) => name !== "app" && (s.profiles ?? []).length === 0)
    .map(([name]) => name);
  rule(unprofiled.length === 0, `only app may run without a profile: ${unprofiled.join(", ")}`);

  const lk = services.livekit;
  rule(
    lk !== undefined && JSON.stringify(lk.profiles ?? []) === JSON.stringify(["voice"]),
    "livekit must exist and be only in the voice profile",
  );
  rule(PINNED.test(lk?.image ?? ""), `livekit image must be pinned to an exact vX.Y.Z: ${lk?.image ?? "(none)"}`);
  const env = Array.isArray(lk?.environment)
    ? Object.fromEntries(lk.environment.filter((e) => typeof e === "string").map((e) => [e.slice(0, e.indexOf("=")), e.slice(e.indexOf("=") + 1)]))
    : (lk?.environment ?? {});
  rule(env.LIVEKIT_KEYS === LIVEKIT_KEYS, `livekit LIVEKIT_KEYS must be ${JSON.stringify(LIVEKIT_KEYS)}`);
  rule(!("keys" in livekit) && !("key_file" in livekit), "livekit.yaml must hold no keys");

  return { services: Object.keys(services).length, rules, problems };
}

const here = dirname(fileURLToPath(import.meta.url));
export function checkDir(dir: string): Result {
  return check(readFileSync(resolve(dir, "compose.yaml"), "utf8"), readFileSync(resolve(dir, "livekit.yaml"), "utf8"));
}

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  const r = checkDir(process.argv[2] ? resolve(process.argv[2]) : here);
  for (const p of r.problems) console.error(`problem: ${p}`);
  console.log(`deploy check: ${r.services} services, ${r.rules} rules; ${r.problems.length} problem(s)`);
  process.exit(r.problems.length === 0 ? 0 : 1);
}
