/**
 * Checks Rubato's stack files (#315): only `app` runs without a profile, LiveKit runs only under
 * the `voice` profile at a pinned version, and its keys come from the names Rubato signs with,
 * never from its config file. The Foundry account (deploy/azure/foundry.bicep) pins its models, keeps
 * key auth on and never outputs its key. Caddy (ADR 0090) runs only under the `public` profile at
 * a pinned version, with public certificates, in front of an app whose own port a public host
 * binds to loopback. Run with `pnpm deploy:check`; tested by `compose.test.ts`.
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "yaml";

type Service = { profiles?: string[]; image?: string; environment?: Record<string, string> | string[]; ports?: string[]; networks?: string[] };
type Network = { ipam?: { config?: { subnet?: string }[] } };
type Compose = { services?: Record<string, Service>; networks?: Record<string, Network> };

/** What Rubato signs join tokens with (ADR 0073), as LiveKit's `key: secret` line. */
export const LIVEKIT_KEYS = "${RUBATO_LIVEKIT_API_KEY}: ${RUBATO_LIVEKIT_API_SECRET}";
const PINNED = /^livekit\/livekit-server:v\d+\.\d+\.\d+$/;
const PINNED_CADDY = /^caddy:\d+\.\d+\.\d+(-alpine)?$/;
/** How the app's port is published: loopback only on a host that faces the internet (ADR 0090). */
export const APP_PORT = "${RUBATO_APP_BIND:-0.0.0.0}:8081:8080";
/** A model version as Azure names one: `2025-12-15` or `001`, never empty or `latest`. */
const PINNED_MODEL = /^\d[\d-]*$/;

export type Result = { services: number; rules: number; problems: string[] };

export function check(composeText: string, livekitText: string, speechText: string, caddyText: string): Result {
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

  // TLS for a server on the internet (ADR 0090): Caddy, only under the `public` profile.
  const caddy = services.caddy;
  rule(
    caddy !== undefined && JSON.stringify(caddy.profiles ?? []) === JSON.stringify(["public"]),
    "caddy must exist and be only in the public profile",
  );
  rule(PINNED_CADDY.test(caddy?.image ?? ""), `caddy image must be pinned to an exact X.Y.Z: ${caddy?.image ?? "(none)"}`);
  rule(
    JSON.stringify(app?.ports ?? []) === JSON.stringify([APP_PORT]),
    `app must publish its port only as ${APP_PORT}, so a public host can bind it to loopback`,
  );
  const subnet = compose.networks?.edge?.ipam?.config?.[0]?.subnet;
  rule(
    typeof subnet === "string" && /^\d+\.\d+\.\d+\.\d+\/\d+$/.test(subnet) && (app?.networks ?? []).includes("edge") && JSON.stringify(caddy?.networks ?? []) === JSON.stringify(["edge"]),
    "the edge network must have a fixed subnet and hold caddy and app, so RUBATO_TRUSTED_PROXY can name it",
  );
  const site = caddyText.replace(/#.*$/gm, "");
  rule(
    /^\{\$RUBATO_DOMAIN\}\s*\{/m.test(site) && /\breverse_proxy\s+app:8080\b/.test(site),
    "Caddyfile must serve {$RUBATO_DOMAIN} and proxy to app:8080",
  );
  rule(
    !/\btls\s+internal\b/.test(site) && !/\bauto_https\s+(off|disable_redirects|disable_certs)\b/.test(site) && !/^\s*http:\/\//m.test(site) && !/\btrusted_proxies\b/.test(site),
    "Caddyfile must keep public certificates and the https redirect, serve no http:// site and trust no proxy in front",
  );

  // The Foundry account is Bicep, read as text: no Bicep parser in Node.
  const code = speechText.replace(/(^|\s)\/\/.*$/gm, "$1");
  rule(!/listKeys/i.test(code), "foundry.bicep must not read the account key (listKeys)");
  const versions = [...code.matchAll(/^\s*version:\s*'([^']*)'/gm)].map((m) => m[1]);
  const loose = versions.filter((v) => !PINNED_MODEL.test(v));
  rule(versions.length > 0 && loose.length === 0, `foundry.bicep must pin every model version: ${versions.length === 0 ? "(none)" : loose.join(", ")}`);
  rule(/^\s*disableLocalAuth:\s*false\b/m.test(code), "foundry.bicep must keep key auth on (disableLocalAuth: false)");

  return { services: Object.keys(services).length, rules, problems };
}

const here = dirname(fileURLToPath(import.meta.url));
export function checkDir(dir: string): Result {
  const read = (f: string) => readFileSync(resolve(dir, f), "utf8");
  return check(read("compose.yaml"), read("livekit.yaml"), read("azure/foundry.bicep"), read("Caddyfile"));
}

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  const r = checkDir(process.argv[2] ? resolve(process.argv[2]) : here);
  for (const p of r.problems) console.error(`problem: ${p}`);
  console.log(`deploy check: ${r.services} services, ${r.rules} rules; ${r.problems.length} problem(s)`);
  process.exit(r.problems.length === 0 ? 0 : 1);
}
