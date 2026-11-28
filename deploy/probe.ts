#!/usr/bin/env -S node --experimental-strip-types
/**
 * Probes a Rubato host from outside, after a deploy on the internet (ADR 0090, plan 345): what
 * `check.ts` says about the files, this says about the server that answers.
 *
 *   pnpm deploy:probe https://rubato.example.org
 *
 * Checks: https answers with Strict-Transport-Security; plain http is redirected to https or
 * closed; nothing answers without a session (401), and what does not exist is 404 either way;
 * the paths of before tenants are gone; an unsigned webhook is not taken; a burst on the pairing
 * path is answered 429 with Retry-After; the app's own port is closed. It reads nothing that
 * needs a login and writes nothing. The burst spends the pairing bucket of the address it runs
 * from for about half a minute. `probe.test.ts` makes every check fail on purpose.
 */
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export type Answer = { status: number; headers: Record<string, string> } | { error: string };
export type Ask = (method: string, url: string, body?: string) => Promise<Answer>;
export type Result = { checks: number; problems: string[] };

/** More than the anonymous burst (`kernel::limits::ANONYMOUS_BURST`), so the last ones are refused. */
export const BURST = 45;
/** The port `compose.yaml` publishes the app at; on a public host only on loopback. */
export const APP_PORT = 8081;

const status = (a: Answer) => ("error" in a ? `no answer (${a.error})` : String(a.status));

export async function probe(base: string, ask: Ask): Promise<Result> {
  const problems: string[] = [];
  let checks = 0;
  const rule = (ok: boolean, problem: string) => {
    checks += 1;
    if (!ok) problems.push(problem);
  };
  const url = new URL(base);
  rule(url.protocol === "https:", `the URL to probe must be https: ${base}`);
  const https = `https://${url.host}`;
  const http = `http://${url.hostname}`;

  const health = await ask("GET", `${https}/api/health`);
  rule(!("error" in health) && health.status === 200, `GET ${https}/api/health must answer 200: ${status(health)}`);
  const hsts = "error" in health ? "" : (health.headers["strict-transport-security"] ?? "");
  rule(/max-age=\d{7,}/.test(hsts), `https answers must carry Strict-Transport-Security with a long max-age: ${hsts || "(none)"}`);
  rule(!("error" in health) && health.headers["x-content-type-options"] === "nosniff" && Boolean(health.headers["content-security-policy"]), "https answers must carry the content security policy and nosniff");

  // Plain http: redirected to https by the proxy, or nothing listens. Never served.
  const plain = await ask("GET", `${http}/api/health`);
  const redirected = !("error" in plain) && plain.status >= 301 && plain.status <= 308 && (plain.headers.location ?? "").startsWith("https://");
  rule("error" in plain || redirected, `plain http must be redirected to https or closed: ${status(plain)}`);

  // Without a session nothing of anyone is answered.
  for (const path of ["/api/me", "/api/t/default/workspaces", "/api/t/default/w/default/overview", "/api/t/default/members", "/api/admin/tenants", "/api/pairings"]) {
    const a = await ask("GET", `${https}${path}`);
    rule(!("error" in a) && a.status === 401, `GET ${path} without a session must answer 401: ${status(a)}`);
  }
  const socket = await ask("GET", `${https}/ws/web?t=default`);
  rule(!("error" in socket) && (socket.status === 401 || socket.status === 403), `GET /ws/web without a session must be refused: ${status(socket)}`);
  const graph = await ask("GET", `${https}/api/graph/snapshot?source=team/default`);
  rule(!("error" in graph) && (graph.status === 401 || graph.status === 404), `GET /api/graph/snapshot without a pairing token must answer 401 (404 with the graph off): ${status(graph)}`);

  // The paths of before tenants (ADR 0089) and paths that never were: 404, session or not.
  for (const path of ["/api/overview", "/api/map", "/api/w/default/overview", "/api/no-such-thing"]) {
    const a = await ask("GET", `${https}${path}`);
    rule(!("error" in a) && a.status === 404, `GET ${path} must answer 404: ${status(a)}`);
  }

  // An unsigned delivery is not taken: refused when webhooks are on, unknown when they are off.
  const hook = await ask("POST", `${https}/webhooks/github`, "{}");
  rule(!("error" in hook) && [400, 401, 403, 404].includes(hook.status), `an unsigned POST /webhooks/github must be refused: ${status(hook)}`);

  // A burst on an anonymous path: the bucket of this address empties and says when to return.
  let limited: Answer | null = null;
  let served = 0;
  for (let i = 0; i < BURST; i++) {
    const a = await ask("POST", `${https}/api/pair/poll`, JSON.stringify({ device_code: "probe" }));
    if (!("error" in a) && a.status === 429) {
      limited = a;
      break;
    }
    served += 1;
  }
  rule(limited !== null, `${BURST} requests to /api/pair/poll must meet a 429: all ${served} were served`);
  rule(limited !== null && !("error" in limited) && /^\d+$/.test(limited.headers["retry-after"] ?? ""), "the 429 must carry Retry-After in seconds");

  // The app's own port: only the proxy reaches the app.
  const direct = await ask("GET", `http://${url.hostname}:${APP_PORT}/api/health`);
  rule("error" in direct, `the app's own port ${APP_PORT} must be closed to the outside: ${status(direct)}`);

  return { checks, problems };
}

/** The real thing: no redirects followed, a short timeout, a failure to connect as `error`. */
export const overHttp: Ask = async (method, url, body) => {
  try {
    const res = await fetch(url, {
      method,
      redirect: "manual",
      signal: AbortSignal.timeout(8000),
      ...(body === undefined ? {} : { body, headers: { "content-type": "application/json" } }),
    });
    await res.arrayBuffer().catch(() => undefined);
    return { status: res.status, headers: Object.fromEntries(res.headers.entries()) };
  } catch (e) {
    return { error: e instanceof Error ? (e.cause instanceof Error ? e.cause.message : e.message) : String(e) };
  }
};

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  const base = process.argv[2];
  if (!base) {
    console.error("usage: probe.ts https://<domain>");
    process.exit(2);
  }
  const r = await probe(base, overHttp);
  for (const p of r.problems) console.error(`problem: ${p}`);
  console.log(`deploy probe of ${base}: ${r.checks} checks; ${r.problems.length} problem(s)`);
  process.exit(r.problems.length === 0 ? 0 : 1);
}
