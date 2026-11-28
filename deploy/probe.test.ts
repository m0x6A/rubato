/**
 * The probe passes on a host that answers as a hardened Rubato does, and each of its checks
 * fails on purpose on a host that answers as an unhardened one would. Run with `pnpm deploy:test`.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { APP_PORT, BURST, probe, type Answer, type Ask } from "./probe.ts";

const SECURE = { "strict-transport-security": "max-age=31536000; includeSubDomains", "x-content-type-options": "nosniff", "content-security-policy": "default-src 'self'" };

/** A host as ADR 0090 leaves it. `change` lets a test break one thing. */
function good(change: (method: string, url: URL, polls: number) => Answer | undefined = () => undefined): Ask {
  let polls = 0;
  return async (method, raw) => {
    const url = new URL(raw);
    if (url.pathname === "/api/pair/poll") polls += 1;
    const changed = change(method, url, polls);
    if (changed) return changed;
    if (url.port === String(APP_PORT)) return { error: "ECONNREFUSED" };
    if (url.protocol === "http:") return { status: 308, headers: { location: `https://${url.hostname}${url.pathname}` } };
    const answer = (status: number, extra: Record<string, string> = {}): Answer => ({ status, headers: { ...SECURE, ...extra } });
    if (url.pathname === "/api/health") return answer(200);
    if (url.pathname === "/api/pair/poll") return polls > 30 ? answer(429, { "retry-after": "1" }) : answer(404);
    if (url.pathname === "/webhooks/github") return answer(401);
    if (["/api/overview", "/api/map", "/api/w/default/overview", "/api/no-such-thing"].includes(url.pathname)) return answer(404);
    return answer(401);
  };
}

test("a hardened host passes every check", async () => {
  const r = await probe("https://rubato.example.org", good());
  assert.deepEqual(r.problems, []);
  console.log(`probed with ${r.checks} checks`);
});

test("must fail: each thing an unhardened host does is reported", async () => {
  const cases: [string, Parameters<typeof good>[0], RegExp][] = [
    ["no HSTS", (_m, u) => (u.pathname === "/api/health" && u.protocol === "https:" ? { status: 200, headers: { "x-content-type-options": "nosniff", "content-security-policy": "x" } } : undefined), /Strict-Transport-Security/],
    ["a short HSTS", (_m, u) => (u.pathname === "/api/health" && u.protocol === "https:" ? { status: 200, headers: { ...SECURE, "strict-transport-security": "max-age=60" } } : undefined), /Strict-Transport-Security/],
    ["no policy headers", (_m, u) => (u.pathname === "/api/health" && u.protocol === "https:" ? { status: 200, headers: { "strict-transport-security": SECURE["strict-transport-security"] } } : undefined), /content security policy/],
    ["plain http served", (_m, u) => (u.protocol === "http:" && !u.port ? { status: 200, headers: {} } : undefined), /plain http must be redirected/],
    ["http redirected to http", (_m, u) => (u.protocol === "http:" && !u.port ? { status: 301, headers: { location: "http://elsewhere/" } } : undefined), /plain http must be redirected/],
    ["a tenant's data without a session", (_m, u) => (u.pathname === "/api/t/default/w/default/overview" ? { status: 200, headers: SECURE } : undefined), /overview without a session must answer 401/],
    ["the operator's routes without a session", (_m, u) => (u.pathname === "/api/admin/tenants" ? { status: 200, headers: SECURE } : undefined), /\/api\/admin\/tenants without a session/],
    ["the socket without a session", (_m, u) => (u.pathname === "/ws/web" ? { status: 101, headers: SECURE } : undefined), /\/ws\/web without a session/],
    ["the graph without a token", (_m, u) => (u.pathname === "/api/graph/snapshot" ? { status: 200, headers: SECURE } : undefined), /snapshot without a pairing token/],
    ["a path of before tenants still served", (_m, u) => (u.pathname === "/api/overview" ? { status: 401, headers: SECURE } : undefined), /GET \/api\/overview must answer 404/],
    ["an unsigned webhook taken", (_m, u) => (u.pathname === "/webhooks/github" ? { status: 200, headers: SECURE } : undefined), /unsigned POST/],
    ["no rate limit", (_m, u) => (u.pathname === "/api/pair/poll" ? { status: 404, headers: SECURE } : undefined), new RegExp(`all ${BURST} were served`)],
    ["a 429 that does not say when", (_m, u, polls) => (u.pathname === "/api/pair/poll" && polls > 30 ? { status: 429, headers: SECURE } : undefined), /Retry-After/],
    ["the app's port open", (_m, u) => (u.port === String(APP_PORT) ? { status: 200, headers: {} } : undefined), /own port 8081 must be closed/],
    ["the host down", () => ({ error: "ENOTFOUND" }), /api\/health must answer 200/],
  ];
  for (const [what, change, expected] of cases) {
    const r = await probe("https://rubato.example.org", good(change));
    assert.ok(r.problems.some((p) => expected.test(p)), `${what}: ${JSON.stringify(r.problems)}`);
  }
  const plain = await probe("http://rubato.example.org", good());
  assert.ok(plain.problems.some((p) => /must be https/.test(p)));
  console.log(`checked ${cases.length + 1} unhardened hosts`);
});
