#!/usr/bin/env -S node --experimental-strip-types
/**
 * A stand-in for github.com and api.github.com in the browser tests of the GitHub App login
 * (ADR 0022). It knows one App (id 12, client id Iv1.e2e) with one installation (34) on
 * m0x6A/rubato, and three accounts:
 *
 *   member    can access installation 34
 *   outsider  can access no installation
 *   broken    the installation lookup answers 500
 *
 * Calls made as the App must carry a JWT signed with the App's key (RS256, checked against
 * the public key given here) and issued by app id 12. Calls made as the installation must
 * carry the token this stand-in minted. So a wrong key or a missing token fails here as it
 * would at GitHub.
 *
 *   github-standin.ts --port <n> --public-key <pem file>
 */
import { createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

const argv = process.argv.slice(2);
const opt = (name: string) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
};
const port = Number(opt("--port") ?? 18092);
const keyPath = opt("--public-key");
if (!keyPath) {
  console.error("usage: github-standin.ts --port <n> --public-key <pem file>");
  process.exit(2);
}
const publicKey = createPublicKey(readFileSync(keyPath, "utf8"));

const APP_ID = "12";
const CLIENT_ID = "Iv1.e2e";
const CLIENT_SECRET = "e2e-secret";
const INSTALLATION = 34;
const INSTALLATION_TOKEN = "ghs_e2e_installation";
const ACCOUNTS: Record<string, { id: number; installations: number[] | "error" }> = {
  member: { id: 1001, installations: [INSTALLATION] },
  outsider: { id: 1002, installations: [] },
  broken: { id: 1003, installations: "error" },
};
const ISSUES = [
  { number: 911, title: "Listed as the App installation", html_url: "https://github.com/m0x6A/rubato/issues/911", labels: [{ name: "bug" }], updated_at: "2026-11-01T10:00:00Z" },
  { number: 912, title: "A pull request, not an issue", html_url: "https://github.com/m0x6A/rubato/pull/912", labels: [], updated_at: "2026-11-01T10:00:00Z", pull_request: {} },
];

/** Calls by path, so the tests can ask what the server did. */
const calls: Record<string, number> = {};

function send(res: ServerResponse, status: number, body: unknown, type = "application/json"): void {
  res.writeHead(status, { "content-type": type });
  res.end(typeof body === "string" ? body : JSON.stringify(body));
}

function bearer(req: IncomingMessage): string {
  return (req.headers.authorization ?? "").replace(/^(Bearer|token)\s+/i, "");
}

/** Is this a JWT the App signed with its key, issued by the App, not expired? */
function isAppJwt(token: string): boolean {
  const parts = token.split(".");
  if (parts.length !== 3) {
    return false;
  }
  try {
    const ok = verify("RSA-SHA256", Buffer.from(`${parts[0]}.${parts[1]}`), publicKey, Buffer.from(parts[2]!, "base64url"));
    const claims = JSON.parse(Buffer.from(parts[1]!, "base64url").toString("utf8")) as { iss?: unknown; exp?: number };
    return ok && String(claims.iss) === APP_ID && (claims.exp ?? 0) > Date.now() / 1000;
  } catch {
    return false;
  }
}

function accountOf(req: IncomingMessage): string | undefined {
  const login = bearer(req).replace(/^user-token-/, "");
  return login in ACCOUNTS ? login : undefined;
}

async function body(req: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const c of req) {
    chunks.push(c as Buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

const escapeHtml = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? "/", `http://127.0.0.1:${port}`);
  const path = url.pathname;
  calls[path] = (calls[path] ?? 0) + 1;
  const unauthorized = () => send(res, 401, { message: "Bad credentials" });

  // github.com: the consent page and the code exchange.
  if (path === "/login/oauth/authorize") {
    if (url.searchParams.get("client_id") !== CLIENT_ID) {
      return send(res, 404, "unknown client id", "text/plain");
    }
    const redirect = url.searchParams.get("redirect_uri") ?? "";
    const state = url.searchParams.get("state") ?? "";
    const links = Object.keys(ACCOUNTS)
      .map((login) => {
        const to = new URL(redirect);
        to.searchParams.set("code", `code-${login}`);
        to.searchParams.set("state", state);
        return `<li><a href="${escapeHtml(to.toString())}">Continue as ${login}</a></li>`;
      })
      .join("");
    return send(res, 200, `<!doctype html><title>GitHub stand-in</title><h1>GitHub stand-in</h1><ul>${links}</ul>`, "text/html");
  }
  if (path === "/login/oauth/access_token" && req.method === "POST") {
    const form = new URLSearchParams(await body(req));
    const basic = Buffer.from((req.headers.authorization ?? "").replace(/^Basic\s+/i, ""), "base64").toString("utf8");
    const secretOk = form.get("client_secret") === CLIENT_SECRET || basic === `${CLIENT_ID}:${CLIENT_SECRET}`;
    const login = (form.get("code") ?? "").replace(/^code-/, "");
    if (!secretOk || !(login in ACCOUNTS)) {
      return send(res, 200, { error: "bad_verification_code", error_description: "The code passed is incorrect or expired." });
    }
    return send(res, 200, { access_token: `user-token-${login}`, token_type: "bearer", scope: "" });
  }

  // api.github.com as the logged-in person.
  if (path === "/user") {
    const login = accountOf(req);
    return login ? send(res, 200, { login, id: ACCOUNTS[login]!.id, avatar_url: null }) : unauthorized();
  }
  if (path === "/user/installations") {
    const login = accountOf(req);
    if (!login) {
      return unauthorized();
    }
    const mine = ACCOUNTS[login]!.installations;
    if (mine === "error") {
      return send(res, 500, { message: "Server Error" });
    }
    return send(res, 200, { total_count: mine.length, installations: mine.map((id) => ({ id, app_id: Number(APP_ID) })) });
  }

  // api.github.com as the App.
  if (path === "/app") {
    return isAppJwt(bearer(req)) ? send(res, 200, { id: Number(APP_ID), client_id: CLIENT_ID, slug: "rubato-e2e" }) : unauthorized();
  }
  if (path === `/app/installations/${INSTALLATION}/access_tokens` && req.method === "POST") {
    if (!isAppJwt(bearer(req))) {
      return unauthorized();
    }
    return send(res, 201, { token: INSTALLATION_TOKEN, expires_at: new Date(Date.now() + 3600_000).toISOString(), permissions: { issues: "read", contents: "write" } });
  }

  // api.github.com as the installation.
  if (path === "/installation/repositories" || path === "/rate_limit" || path.startsWith("/repos/")) {
    if (bearer(req) !== INSTALLATION_TOKEN) {
      return unauthorized();
    }
    if (path === "/installation/repositories") {
      return send(res, 200, { total_count: 1, repositories: [{ full_name: "m0x6A/rubato" }] });
    }
    if (path === "/rate_limit") {
      return send(res, 200, { resources: { core: { remaining: 4900, limit: 5000 } } });
    }
    if (path === "/repos/m0x6A/rubato/issues") {
      return send(res, 200, url.searchParams.get("page") === "1" ? ISSUES : []);
    }
    if (path === "/repos/m0x6A/rubato/branches") {
      return send(res, 200, []);
    }
  }

  // For the tests only.
  if (path === "/_standin/calls") {
    return send(res, 200, calls);
  }
  return send(res, 404, { message: "Not Found", documentation_url: "https://docs.github.com/rest" });
});

server.listen(port, "127.0.0.1", () => console.log(`github stand-in on http://127.0.0.1:${port}`));
