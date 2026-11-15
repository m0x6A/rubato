/**
 * `plan.ts handoff` (ADR 0055, plan 122 design §D7) on a temporary checkout with a bare origin:
 * the approval record, the ADRs accepted at their blobs, one commit with both trailers, the push,
 * every refusal writing nothing, a failed push and its rerun, a repo that is not the map repo, and
 * the report posted to a stub companion.
 */
import { execFileSync } from "node:child_process";
import { createServer, type IncomingHttpHeaders } from "node:http";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";
import { bundleHash, bundlePaths } from "../../../actions/plan-check/src/bundle.ts";
import { acceptedText, pinAdrs, rubatoAcceptance } from "../scripts/adrs.ts";
import { install } from "../scripts/install.ts";
import { HANDOFF_EXIT, type HandoffArgs, handoff, parseHandoffArgs, withApprovalRecord } from "../scripts/lib/handoff.ts";
import type { Workspace } from "../scripts/lib/local.ts";
import { gitBlobSha, planSha } from "../scripts/lib/plan-sha.ts";

const root = resolve(import.meta.dirname, "../../..");
const APPROVAL = "5d3b1f0e-2c4a-4b8e-9f1d-7a6c5e4b3a21";
const URL = "https://rubato.example/approvals/5d3b1f0e";
const PLAN = "docs/plans/7.md";
const DESIGN = "docs/plans/7-try/design.md";
const ADR = "docs/decisions/0099-try-it.md";

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

interface World {
  work: string;
  bare: string;
  ws: Workspace;
  /** The record the companion would type, for the draft as it is now (as it mirrors it: regular files inside the worktree only). */
  args: () => HandoffArgs;
}

const ADR_TEXT = "# 0099. Try it\n\n- **Status:** proposed\n- **Date:** 2026-11-14\n- **Decided by:** agent (test); a human accepts\n- **Seam:** none\n- **Issue:** acme/map#7\n\n## Decision\n\nTry it.\n";

/**
 * A checkout of `origin` (a bare repo at `<tmp>/<repo>.git`) on `issue/7-try`, with the plan, its
 * design note and ADR 0099 drafted and uncommitted, as Rubato mode leaves them. `mapRepo` is the
 * workspace's map: the checkout itself, or a separate map clone.
 */
async function makeWorld(origin = "acme/map", mapRepo = origin): Promise<World> {
  const tmp = mkdtempSync(join(tmpdir(), "rubato-handoff-"));
  const bare = join(tmp, `${origin}.git`);
  mkdirSync(bare, { recursive: true });
  execFileSync("git", ["init", "-q", "--bare", "-b", "main", bare]);
  const work = join(tmp, "work");
  mkdirSync(work);
  git(work, "init", "-q", "-b", "main");
  git(work, "config", "user.name", "agent");
  git(work, "config", "user.email", "agent@example.com");
  git(work, "remote", "add", "origin", bare);
  mkdirSync(join(work, "docs/decisions"), { recursive: true });
  mkdirSync(join(work, "docs/plans/7-try"), { recursive: true });
  writeFileSync(join(work, "README.md"), "x\n");
  let mapLocal = work;
  if (mapRepo === origin) {
    writeFileSync(join(work, "system-map.yml"), readFileSync(join(root, "docs/design/rubato-map.yml"), "utf8"));
  } else {
    mapLocal = join(tmp, "map");
    mkdirSync(mapLocal);
    writeFileSync(join(mapLocal, "system-map.yml"), readFileSync(join(root, "docs/design/rubato-map.yml"), "utf8"));
    git(mapLocal, "init", "-q", "-b", "main");
    git(mapLocal, "-c", "user.name=t", "-c", "user.email=t@t", "add", "-A");
    git(mapLocal, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "map");
  }
  git(work, "add", "-A");
  git(work, "commit", "-q", "-m", "init");
  git(work, "push", "-q", "origin", "main");
  git(work, "checkout", "-q", "-b", "issue/7-try");
  const valid = readFileSync(join(root, "schema/fixtures/plans/valid.md"), "utf8").replace("issue: m0x6A/rubato#42", `issue: ${origin}#7`);
  writeFileSync(join(work, PLAN), `${valid}\nThe design is in [the note](7-try/design.md).\n\n## ADRs proposed\n\n| ADR | File | Blob |\n| --- | --- | --- |\n| 0099 | ${ADR} | x |\n`);
  writeFileSync(join(work, DESIGN), "# Design\n\nIt works like this.\n");
  writeFileSync(join(work, ADR), ADR_TEXT);
  await pinAdrs(work, PLAN);
  // Work in progress the handoff must leave alone: an edit to a tracked file and a new file.
  writeFileSync(join(work, "README.md"), "x\nwork in progress\n");
  writeFileSync(join(work, "wip.ts"), "export {};\n");
  const clone = { repo: mapRepo, path: "system-map.yml", local: mapLocal };
  const ws: Workspace = { map: clone, maps: [clone], repos: [] };
  const args = (): HandoffArgs => {
    const text = readFileSync(join(work, PLAN), "utf8");
    const top = realpathSync(work);
    const files = bundlePaths(text, PLAN).flatMap((p) => {
      try {
        const real = realpathSync(join(work, p));
        if (!real.startsWith(top + sep) || !statSync(real).isFile()) {
          return [];
        }
        return [[p, readFileSync(real, "utf8")] as [string, string]];
      } catch {
        return [];
      }
    });
    return { approval: APPROVAL, sha: planSha(text), bundle: bundleHash(PLAN, files), by: "m0x6A", at: "2026-11-20T10:00:00Z", self: true, url: URL };
  };
  return { work, bare, ws, args };
}

/** Everything the handoff may not touch when it refuses: the files, the index, the commits. */
function snapshot(w: World): string {
  const files = [PLAN, DESIGN, ADR, "README.md", "wip.ts"].map((f) => readFileSync(join(w.work, f), "utf8"));
  return JSON.stringify([files, git(w.work, "status", "--porcelain"), git(w.work, "rev-parse", "HEAD")]);
}

const quiet = { out: () => {}, err: () => {}, env: {} };

describe("plan.ts handoff: the approval becomes one commit (ADR 0055)", () => {
  it("writes the record, accepts the pinned ADR, commits the plan and its bundle with both trailers, and pushes", async () => {
    const w = await makeWorld();
    const a = w.args();
    const printed: string[] = [];
    const r = await handoff(w.ws, w.work, a, { ...quiet, out: (l) => printed.push(l) });
    expect(r.code).toBe(HANDOFF_EXIT.committed);
    const head = git(w.work, "rev-parse", "HEAD");
    expect(r.line).toEqual({ event: "rubato.plan.handoff", outcome: "committed", approval: APPROVAL, commit: head, adrs_accepted: ["0099"], adrs_left: [] });
    // Exactly one line on stdout, and it is the line.
    expect(printed).toEqual([JSON.stringify(r.line)]);

    const plan = readFileSync(join(w.work, PLAN), "utf8");
    expect(plan).toContain(`approved_by: "m0x6A"\napproved_at: "2026-11-20T10:00:00Z"\napproved_sha: "${a.sha}"\n---\n`);
    // Writing the record keeps the hash it records.
    expect(planSha(plan)).toBe(a.sha);
    const adr = readFileSync(join(w.work, ADR), "utf8");
    expect(adr).toContain("- **Status:** accepted\n");
    expect(adr).toContain(`- **Decided by:** m0x6A, accepted 2026-11-20, approved in Rubato ${URL} (self); drafted by agent (test)\n`);

    // One commit on main, with the plan, the design note and the ADR, and nothing else.
    expect(git(w.work, "rev-list", "--count", "main..HEAD")).toBe("1");
    expect(git(w.work, "log", "-1", "--format=%s")).toBe("Plan #7 approved");
    expect(git(w.work, "log", "-1", "--format=%(trailers:key=Approved-by,valueonly)")).toBe("m0x6A");
    expect(git(w.work, "log", "-1", "--format=%(trailers:key=Rubato-Approval,valueonly)")).toBe(APPROVAL);
    expect(git(w.work, "show", "--name-only", "--format=", "HEAD").split("\n").sort()).toEqual([ADR, DESIGN, PLAN].sort());
    // The work in progress stays out of the commit (the helper trims, so " M" reads "M").
    expect(git(w.work, "status", "--porcelain").split("\n").map((l) => l.trim()).sort()).toEqual(["?? wip.ts", "M README.md"]);
    // Pushed: origin has the branch at that commit.
    expect(git(w.bare, "rev-parse", "refs/heads/issue/7-try")).toBe(head);
  });

  it("a rerun after the commit pushes the same commit and makes no second one", async () => {
    const w = await makeWorld();
    const a = w.args();
    const first = await handoff(w.ws, w.work, a, quiet);
    const again = await handoff(w.ws, w.work, a, quiet);
    expect(again.code).toBe(HANDOFF_EXIT.committed);
    expect(again.line).toMatchObject({ outcome: "committed", commit: first.line.commit, adrs_accepted: ["0099"] });
    expect(git(w.work, "rev-list", "--count", "main..HEAD")).toBe("1");
  });

  it("keys already equal from an earlier run that stopped before its commit: it goes on", async () => {
    const w = await makeWorld();
    const a = w.args();
    const plan = join(w.work, PLAN);
    writeFileSync(plan, withApprovalRecord(readFileSync(plan, "utf8"), { approved_by: a.by, approved_at: a.at, approved_sha: a.sha }));
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.code).toBe(HANDOFF_EXIT.committed);
    expect(readFileSync(plan, "utf8").match(/^approved_by:/gm)).toHaveLength(1);
  });

  it("an ADR this approval already accepted, in a run that stopped before its commit, counts at its pinned blob", async () => {
    const w = await makeWorld();
    const a = w.args();
    const adr = join(w.work, ADR);
    writeFileSync(adr, acceptedText(ADR_TEXT, rubatoAcceptance("m0x6A", "2026-11-20", URL, true)));
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.code).toBe(HANDOFF_EXIT.committed);
    expect(r.line.adrs_accepted).toEqual(["0099"]);
  });
});

describe("plan.ts handoff refuses, writing nothing", () => {
  it("plan_changed: the plan no longer hashes to the approved sha", async () => {
    const w = await makeWorld();
    const a = w.args();
    writeFileSync(join(w.work, PLAN), `${readFileSync(join(w.work, PLAN), "utf8")}\nOne more line.\n`);
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect([r.code, r.line.outcome]).toEqual([HANDOFF_EXIT.plan_changed, "plan_changed"]);
    expect(r.line).not.toHaveProperty("commit");
    expect(snapshot(w)).toBe(before);
  });

  it("bundle_changed: a file of the bundle changed after the approval", async () => {
    const w = await makeWorld();
    const a = w.args();
    writeFileSync(join(w.work, DESIGN), "# Design\n\nIt works differently now.\n");
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect([r.code, r.line.outcome]).toEqual([HANDOFF_EXIT.bundle_changed, "bundle_changed"]);
    expect(snapshot(w)).toBe(before);
  });

  it("adr_changed: an ADR changed after it was pinned, even though the approval saw the change", async () => {
    const w = await makeWorld();
    writeFileSync(join(w.work, ADR), `${ADR_TEXT}\nAn edit after pinning.\n`);
    const a = w.args();
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect([r.code, r.line.outcome]).toEqual([HANDOFF_EXIT.adr_changed, "adr_changed"]);
    expect(r.line.error).toContain("0099");
    expect(snapshot(w)).toBe(before);
  });
});

describe("plan.ts handoff: a failed push", () => {
  it("push_failed keeps the commit; a rerun pushes that same commit", async () => {
    const w = await makeWorld();
    const a = w.args();
    git(w.work, "remote", "set-url", "origin", join(w.bare, "..", "gone.git"));
    const failed = await handoff(w.ws, w.work, a, quiet);
    expect([failed.code, failed.line.outcome]).toEqual([HANDOFF_EXIT.push_failed, "push_failed"]);
    const head = git(w.work, "rev-parse", "HEAD");
    expect(failed.line.commit).toBe(head);
    expect(failed.line.error).toBeTruthy();

    git(w.work, "remote", "set-url", "origin", w.bare);
    const again = await handoff(w.ws, w.work, a, quiet);
    expect(again.line).toMatchObject({ outcome: "committed", commit: head });
    expect(git(w.work, "rev-list", "--count", "main..HEAD")).toBe("1");
    expect(git(w.bare, "rev-parse", "refs/heads/issue/7-try")).toBe(head);
  });
});

describe("plan.ts handoff outside the map repo", () => {
  it("writes no ADR and lists it as left, and still commits the plan and pushes", async () => {
    const w = await makeWorld("acme/pantry", "acme/map");
    const a = w.args();
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.code).toBe(HANDOFF_EXIT.committed);
    expect(r.line).toMatchObject({ outcome: "committed", adrs_accepted: [], adrs_left: ["0099"] });
    expect(readFileSync(join(w.work, ADR), "utf8")).toBe(ADR_TEXT);
    expect(readFileSync(join(w.work, PLAN), "utf8")).toContain('approved_by: "m0x6A"');
    expect(git(w.bare, "rev-parse", "refs/heads/issue/7-try")).toBe(git(w.work, "rev-parse", "HEAD"));
  });
});

describe("plan.ts handoff reports to the companion", () => {
  async function stub(): Promise<{ url: string; got: { method: string | undefined; path: string | undefined; headers: IncomingHttpHeaders; body: string }[]; close: () => Promise<void> }> {
    const got: { method: string | undefined; path: string | undefined; headers: IncomingHttpHeaders; body: string }[] = [];
    const server = createServer((req, res) => {
      let body = "";
      req.on("data", (c: Buffer) => (body += c.toString()));
      req.on("end", () => {
        got.push({ method: req.method, path: req.url, headers: req.headers, body });
        res.writeHead(204).end();
      });
    });
    await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
    const port = (server.address() as AddressInfo).port;
    return { url: `http://127.0.0.1:${port}`, got, close: () => new Promise((r) => server.close(() => r())) };
  }

  it("posts the printed line to $RUBATO_HOOK_URL/handoff with the hook token, as the hooks send it", async () => {
    const s = await stub();
    try {
      const w = await makeWorld();
      const printed: string[] = [];
      const r = await handoff(w.ws, w.work, w.args(), { out: (l) => printed.push(l), err: () => {}, env: { RUBATO_HOOK_URL: `${s.url}/`, RUBATO_HOOK_TOKEN: "tok", RUBATO_LAUNCH_ID: "launch-1" } });
      expect(r.code).toBe(0);
      expect(s.got).toHaveLength(1);
      const [req] = s.got;
      expect(req).toMatchObject({ method: "POST", path: "/handoff", body: printed[0] });
      expect(req?.headers["authorization"]).toBe("Bearer tok");
      expect(req?.headers["content-type"]).toBe("application/json");
      expect(req?.headers["x-rubato-launch"]).toBe("launch-1");
      // The hooks send the token the same way, from the 0600 header file curl reads (ADR 0063):
      // the companion writes that one `Authorization: Bearer` line into it.
      expect(readFileSync(join(root, "skills/rubato/hooks.json"), "utf8")).toContain("-H @{header_file}");
      expect(readFileSync(join(root, "crates/rubato-companion/src/kernel/config.rs"), "utf8")).toContain('format!("Authorization: Bearer {}\\n", self.token)');
    } finally {
      await s.close();
    }
  });

  it("a refusal is reported too; without the variables it only prints; a companion that is down changes no exit code", async () => {
    const s = await stub();
    try {
      const w = await makeWorld();
      const a = w.args();
      writeFileSync(join(w.work, PLAN), `${readFileSync(join(w.work, PLAN), "utf8")}\nedit\n`);
      const env = { RUBATO_HOOK_URL: s.url, RUBATO_HOOK_TOKEN: "tok" };
      const refused = await handoff(w.ws, w.work, a, { ...quiet, env });
      expect(refused.code).toBe(HANDOFF_EXIT.plan_changed);
      expect(JSON.parse(s.got[0]?.body ?? "{}")).toMatchObject({ outcome: "plan_changed", approval: APPROVAL });
      await handoff(w.ws, w.work, a, { ...quiet, env: {} });
      await handoff(w.ws, w.work, a, { ...quiet, env: { RUBATO_HOOK_URL: s.url } });
      expect(s.got).toHaveLength(1);
      const errs: string[] = [];
      const down = await handoff(w.ws, w.work, a, { out: () => {}, err: (l) => errs.push(l), env: { RUBATO_HOOK_URL: "http://127.0.0.1:9", RUBATO_HOOK_TOKEN: "tok" } });
      expect(down.code).toBe(HANDOFF_EXIT.plan_changed);
      expect(errs.some((l) => l.includes('"sent":false'))).toBe(true);
    } finally {
      await s.close();
    }
  });
});

describe("plan.ts handoff: its arguments and exit codes", () => {
  it("reads the command exactly as the companion types it (rubato_protocol::handoff_prompt)", () => {
    const rust = readFileSync(join(root, "crates/rubato-protocol/src/session_message.rs"), "utf8");
    const typed = /p\.contains\("plan\.ts handoff (.*?)"\)/.exec(rust)?.[1];
    expect(typed, "the Rust test of handoff_prompt names the command").toBeTruthy();
    const r = parseHandoffArgs((typed as string).split(" "));
    expect(r).toEqual({
      ok: true,
      args: { approval: "00000000-0000-0000-0000-000000000005", sha: "22e89bdc981e97d34dc7057747332a8e23cdf0e9", bundle: "0123abcd", by: "m0x6A", at: "2026-11-15T00:00:00Z", self: true, url: "https://rubato.example/approvals/5" },
    });
    expect(parseHandoffArgs([...(typed as string).split(" ").filter((t) => t !== "--self")])).toMatchObject({ ok: true, args: { self: false } });
  });

  it("refuses missing or malformed arguments, keeping a readable approval for the report", () => {
    expect(parseHandoffArgs(["--sha", "abc"])).toMatchObject({ ok: false });
    const bad = parseHandoffArgs(["--approval", APPROVAL, "--sha", "nothex!", "--bundle", "ab", "--by", "m", "--at", "yesterday", "--url", URL]);
    expect(bad).toMatchObject({ ok: false, approval: APPROVAL });
    expect((bad as { error: string }).error).toMatch(/--sha.*--at|--at.*--sha/);
  });

  it("each outcome has its own exit code, and the outcomes are HandoffOutcome's", () => {
    const rust = readFileSync(join(root, "crates/rubato-protocol/src/handoff.rs"), "utf8");
    const block = /impl HandoffOutcome \{[\s\S]*?\n\}/.exec(rust)?.[0] ?? "";
    const names = [...block.matchAll(/=> "(\w+)"/g)].map((m) => m[1]).sort();
    expect(names.length).toBeGreaterThanOrEqual(6);
    expect(Object.keys(HANDOFF_EXIT).sort()).toEqual(names);
    expect(new Set(Object.values(HANDOFF_EXIT)).size).toBe(Object.keys(HANDOFF_EXIT).length);
  });

  it("the skill's version has handoff: at least SKILL_HANDOFF_VERSION", () => {
    const rust = readFileSync(join(root, "crates/rubato-protocol/src/handoff.rs"), "utf8");
    const need = /SKILL_HANDOFF_VERSION: &str = "([\d.]+)"/.exec(rust)?.[1] ?? "";
    const pkg = JSON.parse(readFileSync(join(root, "skills/rubato/package.json"), "utf8")) as { version: string };
    const parts = (v: string) => v.split(".").map(Number);
    const [have, want] = [parts(pkg.version), parts(need)];
    const cmp = have.map((n, i) => n - (want[i] ?? 0)).find((d) => d !== 0) ?? 0;
    expect(want.length, need).toBe(3);
    expect(cmp, `${pkg.version} has no handoff (needs ${need})`).toBeGreaterThanOrEqual(0);
  });
});

describe("the two planning modes in the skill and the prompt (ADR 0053)", () => {
  const read = (f: string) => readFileSync(join(root, "skills/rubato", f), "utf8");
  const placeholders = (t: string) => [...new Set([...t.matchAll(/\{(\w+)\}/g)].map((m) => m[1]))].sort();

  it("prompt-rubato.md takes the same placeholders as prompt.md, so the server renders either with the same values", () => {
    expect(placeholders(read("prompt-rubato.md"))).toEqual(placeholders(read("prompt.md")));
    expect(placeholders(read("prompt.md"))).toContain("plan_path");
    // Fails on purpose: a variant that lost a placeholder differs.
    expect(placeholders(read("prompt-rubato.md").replace("{plan_path}", "the plan"))).not.toEqual(placeholders(read("prompt.md")));
  });

  it("the Rubato prompt hands off and comments nothing; the git prompt is as it was", () => {
    const rubato = read("prompt-rubato.md");
    for (const s of ["RUBATO_PLANNING=rubato", "plan.ts handoff", "first line `Review:`", "first line `Summary:`", "plan.ts adrs-pin", "uncommitted"]) {
      expect(rubato, s).toContain(s);
    }
    expect(rubato).not.toMatch(/commit it on this branch|as a `Review:` comment/);
    // After the cutover a git-mode launch (a companion that cannot plan in Rubato) pushes and asks for Import.
    expect(read("prompt.md")).toContain("commit it on this branch and push it");
    expect(read("prompt.md")).toContain("import it in Rubato");
    expect(read("prompt.md")).not.toContain("`Review:` comment");
  });

  it("SKILL.md's Rubato mode pins ADRs before the Summary: reply and lists every handoff exit code", () => {
    const skill = read("SKILL.md");
    const mode = skill.slice(skill.indexOf("## 9. Rubato mode (`RUBATO_PLANNING=rubato`)"), skill.indexOf("## Don't"));
    expect(mode.length).toBeGreaterThan(200);
    expect(mode.indexOf("plan.ts adrs-pin")).toBeGreaterThan(-1);
    expect(mode.indexOf("plan.ts adrs-pin")).toBeLessThan(mode.indexOf("first line `Summary:`"));
    for (const [outcome, code] of Object.entries(HANDOFF_EXIT)) {
      expect(mode, outcome).toContain(`| ${code} | \`${outcome}\` |`);
    }
    // After the cutover nothing posts a review, an approval or a decision as a comment, and the
    // wait and the comment acceptance are retired (ADR 0054).
    expect(skill).not.toContain("gh issue comment");
    expect(skill).not.toContain("plan.ts wait docs/plans/<n>.md");
    expect(skill).toContain("`plan.ts wait` and `adrs.ts accept --comment` are retired");
    expect(skill).toContain("press *Import*");
  });
});

describe("plan.ts handoff from an installed copy, as the session runs it", () => {
  it("hands off with the vendored bundle code and exits 0; bad arguments exit 2 with an error line", async () => {
    const to = join(mkdtempSync(join(tmpdir(), "rubato-install-")), "rubato");
    install(to, false);
    symlinkSync(join(root, "skills/rubato/node_modules"), join(to, "node_modules"));
    const w = await makeWorld();
    const a = w.args();
    const yml = join(w.work, "..", "workspace.yml");
    writeFileSync(yml, `map:\n  repo: acme/map\n  path: system-map.yml\n  local: ${w.work}\nrepos: []\n`);
    const env: NodeJS.ProcessEnv = { ...process.env, RUBATO_WORKSPACE: yml, RUBATO_LOCAL: join(w.work, "..", "none.yml") };
    delete env["RUBATO_HOOK_URL"];
    delete env["RUBATO_HOOK_TOKEN"];
    const run = (args: string[]) => {
      try {
        return { code: 0, out: execFileSync("node", ["--experimental-strip-types", join(to, "scripts/plan.ts"), "handoff", ...args], { cwd: w.work, encoding: "utf8", env, stdio: ["ignore", "pipe", "pipe"] }) };
      } catch (e) {
        const x = e as { status: number; stdout: string };
        return { code: x.status, out: x.stdout };
      }
    };
    const ok = run(["--approval", a.approval, "--sha", a.sha, "--bundle", a.bundle, "--by", a.by, "--at", a.at, "--self", "--url", a.url]);
    expect(ok.code).toBe(0);
    const lines = ok.out.trim().split("\n");
    expect(lines).toHaveLength(1);
    expect(JSON.parse(lines[0] as string)).toMatchObject({ event: "rubato.plan.handoff", outcome: "committed", adrs_accepted: ["0099"] });
    const bad = run(["--approval", a.approval, "--sha", "zz"]);
    expect(bad.code).toBe(HANDOFF_EXIT.error);
    expect(JSON.parse(bad.out.trim())).toMatchObject({ outcome: "error", approval: APPROVAL });
  });
});

describe("plan.ts handoff keeps ADR paths in the decisions folder (#210)", () => {
  it("never reads or writes a pinned row outside docs/decisions, lists it as left, and still accepts the one inside", async () => {
    const w = await makeWorld();
    const proposed = (n: string) => ADR_TEXT.replace("# 0099. Try it", `# ${n}. Outside`);
    // Outside the worktree, inside it but outside docs/decisions, and a symlink in docs/decisions that leads out.
    const outside = join(w.work, "..", "outside.md");
    const loose = join(w.work, "docs/0094-loose.md");
    const target = join(w.work, "..", "target.md");
    writeFileSync(outside, proposed("0093"));
    writeFileSync(loose, proposed("0094"));
    writeFileSync(target, proposed("0095"));
    symlinkSync("../../../target.md", join(w.work, "docs/decisions/0095-link.md"));
    const rows: [string, string, string][] = [
      ["0093", "../outside.md", outside],
      ["0094", "docs/0094-loose.md", loose],
      ["0095", "docs/decisions/0095-link.md", target],
    ];
    const plan = join(w.work, PLAN);
    writeFileSync(plan, `${readFileSync(plan, "utf8")}${rows.map(([n, f, abs]) => `| ${n} | ${f} | ${gitBlobSha(readFileSync(abs, "utf8"))} |\n`).join("")}`);
    const before = rows.map(([, , abs]) => readFileSync(abs, "utf8"));
    const a = w.args();

    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.code).toBe(HANDOFF_EXIT.committed);
    expect(r.line).toMatchObject({ outcome: "committed", adrs_accepted: ["0099"] });
    expect([...r.line.adrs_left].sort()).toEqual(["0093", "0094", "0095"]);
    expect(rows.map(([, , abs]) => readFileSync(abs, "utf8"))).toEqual(before);
    expect(readFileSync(join(w.work, ADR), "utf8")).toContain("- **Status:** accepted\n");

    // A rerun pushes the same commit and still leaves them alone.
    const again = await handoff(w.ws, w.work, a, quiet);
    expect(again.line).toMatchObject({ outcome: "committed", commit: r.line.commit, adrs_accepted: ["0099"] });
    expect([...again.line.adrs_left].sort()).toEqual(["0093", "0094", "0095"]);
    expect(rows.map(([, , abs]) => readFileSync(abs, "utf8"))).toEqual(before);
    console.log(`checked ${rows.length} escaping rows and one inside`);
  });

  it("a row outside the folder never counts at its pinned blob: edited into accepted text after the approval, it is bundle_changed", async () => {
    const w = await makeWorld();
    const loose = join(w.work, "docs/0094-loose.md");
    const text = ADR_TEXT.replace("# 0099. Try it", "# 0094. Loose");
    writeFileSync(loose, text);
    const plan = join(w.work, PLAN);
    writeFileSync(plan, `${readFileSync(plan, "utf8")}| 0094 | docs/0094-loose.md | ${gitBlobSha(text)} |\n`);
    const a = w.args();
    // After the approval the file reads as accepted by it; inside the folder that counts at its pinned blob.
    writeFileSync(loose, text.replace("- **Status:** proposed", "- **Status:** accepted").replace(/- \*\*Decided by:\*\*.*/, `- **Decided by:** m0x6A, accepted 2026-11-20, approved in Rubato ${URL} (self); drafted by agent (test)`));
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.code).toBe(HANDOFF_EXIT.bundle_changed);
    expect(snapshot(w)).toBe(before);
  });
});

describe("plan.ts handoff counts an ADR at its pinned blob only for the text it would write (#212)", () => {
  const accepted = (text: string) => acceptedText(text, rubatoAcceptance("m0x6A", "2026-11-20", URL, true));

  it("an ADR edited after the approval, with the accepted lines set, is bundle_changed and nothing is written", async () => {
    const w = await makeWorld();
    const a = w.args();
    // Status and Decided by read exactly as this approval writes them; the decision text does not.
    writeFileSync(join(w.work, ADR), accepted(ADR_TEXT).replace("Try it.\n", "Do something nobody approved.\n"));
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect([r.code, r.line.outcome]).toEqual([HANDOFF_EXIT.bundle_changed, "bundle_changed"]);
    expect(r.line.error).toContain("ADR 0099 reads as accepted by this approval but its text differs from the acceptance over its pinned text");
    expect(snapshot(w)).toBe(before);
  });

  it("an ADR pinned already accepted, its Decided by rewritten as this approval would, is bundle_changed", async () => {
    const w = await makeWorld();
    const alice = acceptedText(ADR_TEXT, "alice, accepted 2026-11-01");
    writeFileSync(join(w.work, ADR), alice);
    await pinAdrs(w.work, PLAN);
    const a = w.args();
    // The handoff never writes over an ADR that was not proposed; this text claims it did.
    writeFileSync(join(w.work, ADR), accepted(alice));
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.line.outcome).toBe("bundle_changed");
    expect(r.line.error).toContain("was not proposed");
    expect(snapshot(w)).toBe(before);
  });

  it("the handoff keeps the pinned text itself: a run stopped after its ADR write, with the pin's object gone, finishes on the rerun", async () => {
    const w = await makeWorld();
    const a = w.args();
    const pin = gitBlobSha(ADR_TEXT);
    rmSync(join(w.work, ".git/objects", pin.slice(0, 2), pin.slice(2)), { force: true });
    // Stop the first run at its commit, after it wrote the record and the acceptance.
    const hook = join(w.work, ".git/hooks/pre-commit");
    writeFileSync(hook, "#!/bin/sh\nexit 1\n", { mode: 0o755 });
    const first = await handoff(w.ws, w.work, a, quiet);
    expect(first.line.outcome).toBe("error");
    expect(readFileSync(join(w.work, ADR), "utf8")).toBe(accepted(ADR_TEXT));
    rmSync(hook);
    const again = await handoff(w.ws, w.work, a, quiet);
    expect(again.line).toMatchObject({ outcome: "committed", adrs_accepted: ["0099"] });
  });

  it("adrs-pin outside a git checkout still fills the Blob column", async () => {
    const dir = mkdtempSync(join(tmpdir(), "rubato-pin-"));
    mkdirSync(join(dir, "docs/decisions"), { recursive: true });
    mkdirSync(join(dir, "docs/plans"), { recursive: true });
    writeFileSync(join(dir, ADR), ADR_TEXT);
    writeFileSync(join(dir, PLAN), `# Plan\n\n## ADRs proposed\n\n| ADR | File | Blob |\n| --- | --- | --- |\n| 0099 | ${ADR} | x |\n`);
    const rows = await pinAdrs(dir, PLAN);
    expect(rows[0]?.blob).toBe(gitBlobSha(ADR_TEXT));
    expect(readFileSync(join(dir, PLAN), "utf8")).toContain(gitBlobSha(ADR_TEXT));
  });

  it("a ./ row that stopped before its commit is committed on the rerun", async () => {
    const w = await makeWorld();
    const plan = join(w.work, PLAN);
    writeFileSync(plan, readFileSync(plan, "utf8").replace(`| 0099 | ${ADR} |`, `| 0099 | ./${ADR} |`));
    const a = w.args();
    writeFileSync(join(w.work, ADR), accepted(ADR_TEXT));
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.line).toMatchObject({ outcome: "committed", adrs_accepted: ["0099"] });
  });

  it("an accepted ADR whose pinned text is not in git counts at its real blob: bundle_changed", async () => {
    const w = await makeWorld();
    const a = w.args();
    writeFileSync(join(w.work, ADR), accepted(ADR_TEXT));
    // Pinned in another clone: the object for the pinned blob is not in this one.
    const pin = gitBlobSha(ADR_TEXT);
    rmSync(join(w.work, ".git/objects", pin.slice(0, 2), pin.slice(2)), { force: true });
    const before = snapshot(w);
    const r = await handoff(w.ws, w.work, a, quiet);
    expect(r.line.outcome).toBe("bundle_changed");
    expect(snapshot(w)).toBe(before);
  });

  it("adrs-pin keeps each pinned ADR's text as a git object, so a stopped handoff can find it", async () => {
    const w = await makeWorld();
    expect(git(w.work, "cat-file", "-t", gitBlobSha(ADR_TEXT))).toBe("blob");
    expect(git(w.work, "cat-file", "blob", gitBlobSha(ADR_TEXT))).toBe(ADR_TEXT.trim());
  });
});
