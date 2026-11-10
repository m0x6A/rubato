#!/usr/bin/env -S node --experimental-strip-types
/**
 * End to end (docs/plans/v0.md task 7): two connected repos (an auth provider and a consumer),
 * three issues through the flow with the real rubato-server and rubato-companion, a Claude Code
 * stand-in that runs the injected hooks and pushes a plan, an OTLP receiver, and the plan check
 * over the plans that came back. Prints the progress transitions it observed and a summary.
 *
 *   node --experimental-strip-types e2e/run.ts        (after `cargo build --workspace`)
 *   KEEP=1 keeps the temp directory; VERBOSE=1 streams server and companion logs.
 */
import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, globSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const T = `/tmp/rubato-e2e-${process.pid}`;
const BIN = { server: `${ROOT}/target/debug/rubato-server`, companion: `${ROOT}/target/debug/rubato-companion` };
const children: ChildProcess[] = [];
const started = Date.now();
const log = (msg: string) => console.log(`[${((Date.now() - started) / 1000).toFixed(1).padStart(5)}s] ${msg}`);
const fail = (msg: string): never => {
  throw new Error(msg);
};

function sh(cwd: string, cmd: string, args: string[], env: Record<string, string> = {}): string {
  return execFileSync(cmd, args, { cwd, encoding: "utf8", env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"] }).trim();
}
const git = (cwd: string, ...args: string[]) => sh(cwd, "git", ["-c", "user.name=e2e", "-c", "user.email=e2e@acme.test", ...args]);

function makeRepo(name: string): { clone: string; bare: string } {
  const short = name.split("/")[1] as string;
  const bare = `${T}/origins/${name}.git`;
  mkdirSync(bare, { recursive: true });
  git(bare, "init", "-q", "--bare", "-b", "main");
  const clone = `${T}/repos/${short}`;
  mkdirSync(`${clone}/docs`, { recursive: true });
  git(dirname(clone), "init", "-q", "-b", "main", short);
  writeFileSync(`${clone}/README.md`, `# ${name}\n`);
  writeFileSync(`${clone}/docs/.keep`, "");
  git(clone, "add", "-A");
  git(clone, "commit", "-q", "-m", "init");
  git(clone, "remote", "add", "origin", bare);
  git(clone, "push", "-q", "-u", "origin", "main");
  return { clone, bare };
}

async function waitFor<T>(what: string, timeoutMs: number, probe: () => Promise<T | null | undefined | false>): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const v = await probe().catch(() => null);
    if (v) {
      return v;
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  return fail(`timed out waiting for ${what}`);
}

function spawnLogged(name: string, cmd: string, args: string[], env: Record<string, string>, cwd = T): ChildProcess {
  const child = spawn(cmd, args, { cwd, env: { ...process.env, ...env, TMUX: "" }, stdio: ["ignore", "pipe", "pipe"] });
  const out = `${T}/${name}.log`;
  writeFileSync(out, "");
  for (const stream of [child.stdout, child.stderr]) {
    stream?.on("data", (d) => {
      writeFileSync(out, d, { flag: "a" });
      if (process.env["VERBOSE"]) {
        process.stdout.write(`${name}| ${d}`);
      }
    });
  }
  children.push(child);
  return child;
}

async function main() {
  for (const b of Object.values(BIN)) {
    if (!existsSync(b)) {
      fail(`${b} missing; run cargo build --workspace`);
    }
  }
  for (const d of ["tmux", "state", "origins", "repos", "map", "home"]) {
    mkdirSync(`${T}/${d}`, { recursive: true });
  }

  // 1. The team's map repo and the two connected repos.
  writeFileSync(`${T}/map/system-map.yml`, readFileSync(`${ROOT}/e2e/fixtures/team-map.yml`));
  git(`${T}/map`, "init", "-q", "-b", "main");
  git(`${T}/map`, "add", "-A");
  git(`${T}/map`, "commit", "-q", "-m", "map");
  const auth = makeRepo("acme/auth");
  const pantry = makeRepo("acme/pantry");
  log("repos ready: acme/auth (auth provider), acme/pantry (consumer); map at " + `${T}/map/system-map.yml`);

  // 2. An OTLP receiver: counts posts per path and remembers which rubato.* names it saw, and
  //    which metric names each binary exported (keyed by the service.name string in the body).
  const otlp = { posts: new Map<string, number>(), names: new Set<string>(), metrics: new Map<string, Set<string>>() };
  const receiver = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const body = Buffer.concat(chunks).toString("latin1");
      otlp.posts.set(req.url ?? "?", (otlp.posts.get(req.url ?? "?") ?? 0) + 1);
      for (const m of body.matchAll(/rubato\.[a-z_.]+/g)) {
        otlp.names.add(m[0]);
      }
      const service = ["rubato-server", "rubato-companion"].find((n) => body.includes(n));
      if (req.url === "/v1/metrics" && service) {
        const seen = otlp.metrics.get(service) ?? new Set<string>();
        for (const m of body.matchAll(/rubato\.[a-z_.]+/g)) {
          seen.add(m[0]);
        }
        otlp.metrics.set(service, seen);
      }
      res.writeHead(200, { "content-type": "application/x-protobuf" }).end();
    });
  });
  await new Promise<void>((ok) => receiver.listen(0, "127.0.0.1", () => ok()));
  const otlpUrl = `http://127.0.0.1:${(receiver.address() as { port: number }).port}`;

  // 3. The server, reading branches and plans from the local bare repos.
  const port = 18100 + (process.pid % 400);
  const url = `http://127.0.0.1:${port}`;
  spawnLogged("server", BIN.server, [], {
    RUBATO_BIND: `127.0.0.1:${port}`,
    RUBATO_PUBLIC_URL: url,
    RUBATO_DEV_USER: "m0x6A",
    RUBATO_DATABASE_URL: `sqlite://${T}/server.db?mode=rwc`,
    RUBATO_MAP_FILE: `${T}/map/system-map.yml`,
    RUBATO_LOCAL_REPOS: `acme/auth=${auth.bare},acme/pantry=${pantry.bare}`,
    RUBATO_GITHUB_POLL_SECS: "5",
    RUBATO_WEB_DIST: `${ROOT}/web/dist`,
    OTEL_EXPORTER_OTLP_ENDPOINT: otlpUrl,
    OTEL_METRIC_EXPORT_INTERVAL: "2000",
    OTEL_BSP_SCHEDULE_DELAY: "1000",
    RUST_LOG: "info",
  });
  await waitFor("server health", 20_000, () => fetch(`${url}/api/health`).then((r) => r.ok));
  const api = async <T>(method: string, path: string, body?: unknown): Promise<T> => {
    const init: RequestInit = { method };
    if (body !== undefined) {
      init.headers = { "content-type": "application/json" };
      init.body = JSON.stringify(body);
    }
    const r = await fetch(`${url}${path}`, init);
    if (!r.ok) {
      fail(`${method} ${path}: ${r.status} ${await r.text()}`);
    }
    const text = await r.text();
    return text ? (JSON.parse(text) as T) : (undefined as T);
  };
  log(`server up at ${url}; map loaded from file`);

  // 4. Pair the companion with the real `pair` subcommand: read the code it prints, approve it as the human.
  const pair = spawnLogged("pair", BIN.companion, ["pair", "--server", url, "--state-dir", `${T}/state`], { HOME: `${T}/home` });
  const code = await waitFor("pairing code", 20_000, async () => /code: ([A-Z0-9]{4}-[A-Z0-9]{4})/.exec(readFileSync(`${T}/pair.log`, "utf8"))?.[1]);
  await api("POST", "/api/pair/approve", { user_code: code });
  await new Promise<void>((ok, bad) => pair.on("exit", (c) => (c === 0 ? ok() : bad(new Error(`pair exited ${c}: ${readFileSync(`${T}/pair.log`, "utf8")}`)))));
  log(`companion paired with code ${code}`);

  // 5. The developer's workspace.yml and the companion daemon.
  writeFileSync(
    `${T}/workspace.yml`,
    [
      `server_url: ${url}`,
      "map:",
      "  repo: acme/map",
      "  path: system-map.yml",
      `  local: ${T}/map`,
      "repos:",
      `  - name: acme/auth`,
      `    local: ${auth.clone}`,
      `  - name: acme/pantry`,
      `    local: ${pantry.clone}`,
      "otel:",
      `  endpoint: ${otlpUrl}`,
      "  protocol: http/protobuf",
      "companion:",
      `  claude_command: ${ROOT}/e2e/claude`,
      `  tmux_tmpdir: ${T}/tmux`,
      "  limits:",
      "    heartbeat_interval_secs: 2",
      "    team_poll_interval_secs: 2",
      "    session_stale_secs: 120",
      "    companion_silent_secs: 600",
      "    plan_poll_interval_secs: 60",
      "    github_poll_interval_secs: 60",
      "  pricing:",
      '    "claude-":',
      "      input: 3",
      "      output: 15",
      "      cache_read: 0.3",
      "      cache_write: 3.75",
      "",
    ].join("\n"),
  );
  spawnLogged("companion", BIN.companion, ["run", "--workspace", `${T}/workspace.yml`, "--state-dir", `${T}/state`], {
    HOME: `${T}/home`,
    RUBATO_STANDIN_PLANS: `${ROOT}/e2e/fixtures/plans`,
    OTEL_METRIC_EXPORT_INTERVAL: "2000",
    OTEL_BSP_SCHEDULE_DELAY: "1000",
    RUST_LOG: "info",
  });
  type Overview = { issues: { issue: { repo: string; number: number }; progress: { state: string; reason?: string }; sessions?: { status: string; source: string; spend_usd?: number }[]; plan?: unknown }[]; findings: { kind: string; seam?: string; issues?: { number: number }[] }[]; companions: { connected: boolean; user: string }[] };
  const overview = () => api<Overview>("GET", "/api/overview");
  await waitFor("companion connected", 20_000, async () => (await overview()).companions.some((c) => c.connected && c.user === "m0x6A"));
  log("companion connected");

  // 6. Three issues: pin, launch, and follow the progress model.
  const issues = [
    { repo: "acme/auth", number: 1, target: { kind: "node", id: "auth-service" }, slug: "introspection" },
    { repo: "acme/pantry", number: 1, target: { kind: "node", id: "pantry-api" }, slug: "validate-tokens" },
    { repo: "acme/pantry", number: 2, target: { kind: "seam", id: "pantry-web-api" }, slug: "login-header" },
  ];
  const seen = new Map<string, string[]>();
  for (const i of issues) {
    await api("POST", "/api/pins", { issue: { repo: i.repo, number: i.number }, target: i.target });
    const r = await api<{ command: { branch: string; session_id: string } }>("POST", "/api/launch", { issue: { repo: i.repo, number: i.number }, slug: i.slug });
    log(`launched ${i.repo}#${i.number} on ${r.command.branch} (pinned to ${i.target.kind} ${i.target.id})`);
    seen.set(`${i.repo}#${i.number}`, []);
  }
  const record = (ov: Overview) => {
    for (const s of ov.issues) {
      const key = `${s.issue.repo}#${s.issue.number}`;
      const states = seen.get(key);
      if (states && states[states.length - 1] !== s.progress.state) {
        states.push(s.progress.state);
        log(`${key}: ${s.progress.state}${s.progress.reason ? ` (${s.progress.reason})` : ""}`);
      }
    }
  };
  const final = await waitFor("all three issues Designed", 150_000, async () => {
    const ov = await overview();
    record(ov);
    const done = issues.every((i) => ov.issues.find((s) => s.issue.repo === i.repo && s.issue.number === i.number)?.progress.state === "designed");
    return done ? ov : null;
  });

  // 7. Assertions.
  const problems: string[] = [];
  for (const i of issues) {
    const s = final.issues.find((x) => x.issue.repo === i.repo && x.issue.number === i.number)!;
    const path = seen.get(`${i.repo}#${i.number}`) ?? [];
    if (path[0] !== "planned" || path[path.length - 1] !== "designed") {
      problems.push(`${i.repo}#${i.number} went ${path.join(" → ")}; expected planned first and designed last`);
    }
    if (!s.sessions?.some((x) => x.source === "hook")) {
      problems.push(`${i.repo}#${i.number} has no hook-sourced session`);
    }
    if (!s.sessions?.some((x) => (x.spend_usd ?? 0) > 0)) {
      problems.push(`${i.repo}#${i.number} has no spend from the transcript`);
    }
    if (!s.plan) {
      problems.push(`${i.repo}#${i.number} has no plan`);
    }
  }
  // The overview is sampled every 250 ms, so a short session can pass its live states between
  // two samples; across the three issues the live states must have been seen.
  const allStates = new Set([...seen.values()].flat());
  if (!allStates.has("designing") && !allStates.has("building")) {
    problems.push(`no issue was ever observed designing or building: ${JSON.stringify([...seen])}`);
  }
  const collision = final.findings.find((f) => f.kind === "seam_collision" && f.seam === "auth-contract-pantry");
  if (!collision || (collision.issues?.length ?? 0) !== 2) {
    problems.push(`expected a seam collision on auth-contract-pantry between acme/auth#1 and acme/pantry#1, got ${JSON.stringify(final.findings)}`);
  }
  const standinLogs = globSync(`${T}/repos/*-worktrees/**/.claude-standin.log`);
  if (standinLogs.length !== 3) {
    problems.push(`expected 3 stand-in logs, found ${standinLogs.length}`);
  }
  for (const f of standinLogs) {
    const text = readFileSync(f, "utf8");
    const attrs = /OTEL_RESOURCE_ATTRIBUTES=(.*)/.exec(text)?.[1] ?? "";
    for (const k of ["rubato.repo=", "rubato.issue=", "rubato.branch=", "rubato.node="]) {
      if (!attrs.includes(k)) {
        problems.push(`${f}: OTEL_RESOURCE_ATTRIBUTES lacks ${k} (${attrs})`);
      }
    }
    if (!text.includes("CLAUDE_CODE_ENABLE_TELEMETRY=1") || !text.includes("OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf")) {
      problems.push(`${f}: telemetry env not set for the session`);
    }
    if (!/HOOK Stop exit=0/.test(text)) {
      problems.push(`${f}: Stop hook did not run`);
    }
  }
  // The plans that came back pass the plan check against the team map.
  const plans = globSync(`${T}/repos/*-worktrees/**/docs/plans/*.md`);
  const check = spawn("node", ["--experimental-strip-types", `${ROOT}/actions/plan-check/src/cli.ts`, "--map", `${T}/map/system-map.yml`, ...plans.flatMap((p) => ["--plans", p])], { encoding: "utf8" } as never);
  const checkOut = await new Promise<{ code: number | null; out: string }>((ok) => {
    let out = "";
    check.stdout?.on("data", (d) => (out += d));
    check.stderr?.on("data", (d) => (out += d));
    check.on("exit", (code) => ok({ code, out }));
  });
  const summary = checkOut.out.trim().split("\n").pop() ?? "";
  log(`plan check: ${summary}`);
  if (checkOut.code !== 0 || !summary.includes("checked 3 plan(s)")) {
    problems.push(`plan check over the pushed plans failed: ${checkOut.out}`);
  }
  // OTLP: both binaries exported, and the names are ours. Every metric recorded on the happy
  // path, per binary (failure-only counters are left out on purpose: events.dropped,
  // map.load_failures, plan.parse_failures). A name missing here is a dropped recording site.
  const expectedMetrics: Record<string, string[]> = {
    "rubato-server": [
      "rubato.sessions.by_state",
      "rubato.companion.heartbeat",
      "rubato.ws.clients",
      "rubato.map.loaded",
      "rubato.github.rate_limit.remaining",
      "rubato.github.rate_limit.limit",
      "rubato.issues.by_state",
      "rubato.seam.open_plans",
      "rubato.plan.unknown_id_count",
      "rubato.poll.ticks",
    ],
    "rubato-companion": ["rubato.companion.heartbeat", "rubato.hook.forwarded", "rubato.sessions.discovered", "rubato.sessions.by_state"],
  };
  const missingMetrics = () =>
    Object.entries(expectedMetrics).flatMap(([service, names]) => names.filter((n) => !otlp.metrics.get(service)?.has(n)).map((n) => `${service}: ${n}`));
  await waitFor("OTLP metrics and traces from both binaries", 30_000, async () => missingMetrics().length === 0 && (otlp.posts.get("/v1/traces") ?? 0) > 0).catch((e) =>
    problems.push(`${String(e)}; metric names not seen: ${missingMetrics().join(", ") || "none"}`),
  );
  for (const [service, names] of Object.entries(expectedMetrics)) {
    log(`otlp ${service}: saw ${names.filter((n) => otlp.metrics.get(service)?.has(n)).length}/${names.length} metric names`);
  }
  log(`otlp receiver: ${[...otlp.posts.entries()].map(([p, n]) => `${p}×${n}`).join(", ")}; rubato names: ${otlp.names.size}`);

  console.log("\n=== Rubato v0 end to end ===");
  for (const [k, v] of seen) {
    console.log(`${k.padEnd(16)} ${v.join(" → ")}`);
  }
  console.log(`findings: ${final.findings.map((f) => f.kind + (f.seam ? ` on ${f.seam}` : "")).join("; ") || "none"}`);
  console.log(`sessions with spend: ${final.issues.flatMap((i) => i.sessions ?? []).filter((s) => (s.spend_usd ?? 0) > 0).length}`);
  console.log(`checked: ${issues.length} issues, ${standinLogs.length} launched sessions, ${plans.length} plans, ${otlp.names.size} rubato.* telemetry names`);
  if (problems.length) {
    console.log(`\n${problems.length} problem(s):`);
    for (const p of problems) {
      console.log(`  - ${p}`);
    }
    process.exitCode = 1;
  } else {
    console.log("\nall checks passed");
  }
}

function cleanup() {
  for (const c of children) {
    c.kill("SIGTERM");
  }
  try {
    execFileSync("tmux", ["kill-server"], { env: { ...process.env, TMUX_TMPDIR: `${T}/tmux`, TMUX: "" }, stdio: "ignore" });
  } catch {
    // no server
  }
  if (!process.env["KEEP"]) {
    rmSync(T, { recursive: true, force: true });
  } else {
    console.log(`kept ${T}`);
  }
}

main()
  .catch((e) => {
    console.error(`e2e failed: ${e instanceof Error ? e.message : e}`);
    for (const name of ["server", "companion", "pair"]) {
      const p = `${T}/${name}.log`;
      if (existsSync(p)) {
        console.error(`--- ${name}.log (tail)\n${readFileSync(p, "utf8").split("\n").slice(-25).join("\n")}`);
      }
    }
    process.exitCode = 1;
  })
  .finally(() => {
    cleanup();
    setTimeout(() => process.exit(), 200);
  });
