import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { accept, pinnedAdrs, pinAdrs } from "../scripts/adrs.ts";
import { install } from "../scripts/install.ts";
import { approvalFor, classify, currentApproval, type GhComment, parseApproval, shaToken, trustedAuthor } from "../scripts/lib/comments.ts";
import { gitBlobSha, planSha } from "../scripts/lib/plan-sha.ts";
import type { Workspace } from "../scripts/lib/local.ts";
import { WAIT_EXIT, wait } from "../scripts/plan.ts";

const root = resolve(import.meta.dirname, "../../..");
const fixtures = join(root, "schema/fixtures/comments");

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-C", cwd, "-c", "user.name=t", "-c", "user.email=t@t", ...args], { encoding: "utf8" }).trim();
}

describe("comments.ts reads approvals as the server does", () => {
  it("parses the shared comment fixtures to the same sha and branch as ApprovalRecord::parse_body", () => {
    const read = (f: string) => readFileSync(join(fixtures, f), "utf8");
    expect(parseApproval(read("approved.md"))).toEqual({ sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9", branch: null });
    expect(parseApproval(read("approved-on-branch.md"))).toEqual({ sha: "f3c5e84", branch: "issue/71-plan" });
    expect(parseApproval(read("approved-no-sha.md"))).toEqual({ sha: null, branch: null });
    expect(parseApproval(read("not-a-signal.md"))).toBeNull();
    expect(parseApproval(read("review.md"))).toBeNull();
    expect([shaToken("`ABCDEF1`."), shaToken("abc12"), shaToken("xyz1234")]).toEqual(["abcdef1", null, null]);
  });

  it("trusts the author fixtures exactly as trusted_author does", () => {
    const dir = join(fixtures, "authors");
    const files = readdirSync(dir).filter((f) => f.endsWith(".json"));
    expect(files.length).toBeGreaterThanOrEqual(7);
    for (const f of files) {
      const { trusted, comment } = JSON.parse(readFileSync(join(dir, f), "utf8")) as { trusted: boolean; comment: GhComment };
      expect(trustedAuthor(comment.author_association, comment.user.type === "Bot"), f).toBe(trusted);
    }
    // Fails on purpose: a Bot OWNER expected to count.
    expect(trustedAuthor("OWNER", true)).not.toBe(true);
  });

  it("counts the newest trusted approval, unedited, naming the sha", () => {
    const sha = "1e29c8fac8411763ac280e957033f587924db5c5";
    const c = (id: number, at: string, body: string, extra: Partial<GhComment> = {}): GhComment => ({
      id, body, created_at: at, updated_at: at, html_url: `u${id}`, author_association: "MEMBER", user: { login: "teammate", type: "User" }, ...extra,
    });
    const current = c(1, "2026-11-12T10:00:00Z", `Approved: plan ${sha}`);
    const older = c(2, "2026-11-12T11:00:00Z", "Approved: plan deadbeef");
    expect(approvalFor([current], sha)).toMatchObject({ ok: true });
    expect(approvalFor([current, older], sha)).toEqual({ ok: false, reason: "earlier_sha" });
    expect(approvalFor([c(3, "2026-11-12T10:00:00Z", "Approved: plan 1e29c8f")], sha)).toMatchObject({ ok: true });
    expect(approvalFor([{ ...current, updated_at: "2026-11-12T12:00:00Z" }], sha)).toEqual({ ok: false, reason: "edited" });
    const bot = c(4, "2026-11-12T12:00:00Z", "Approved: plan deadbeef", { user: { login: "x[bot]", type: "Bot" } });
    expect(currentApproval([current, bot])?.id).toBe(1);
    expect(classify(bot, sha)).toBe("untrusted");
    expect(classify(c(5, "t", "Review: full — passed"), sha)).toBe("review");
    expect(classify(c(6, "t", "Decision: x"), sha)).toBe("decision");
    expect(classify(c(7, "t", "lgtm"), sha)).toBeNull();
  });
});

// ----- a map repo with a pushed plan, a stub gh on PATH --------------------------------------

interface World {
  work: string;
  ws: Workspace;
  plan: string;
  adr: string;
  other: string;
  sha: string;
  stub: (s: Stub) => void;
}

interface Stub {
  login?: string;
  comments?: GhComment[];
  issue_url?: string;
  fail?: boolean;
}

const GH = `#!/usr/bin/env node
const fs = require("fs");
const a = process.argv.slice(2);
const st = JSON.parse(fs.readFileSync(process.env.GH_STUB, "utf8"));
fs.appendFileSync(process.env.GH_STUB + ".calls", a.join(" ") + "\\n");
if (st.fail) { process.stderr.write("gh: HTTP 502\\n"); process.exit(1); }
if (a[0] === "api" && a[1] === "user") { console.log(st.login || "me"); process.exit(0); }
const path = a.find((x) => x.startsWith("repos/")) || "";
const one = /issues\\/comments\\/(\\d+)$/.exec(path);
if (one) { const c = (st.comments || []).find((c) => String(c.id) === one[1]); if (!c) process.exit(1); console.log(JSON.stringify({ ...c, issue_url: st.issue_url })); process.exit(0); }
if (/\\/issues\\/\\d+\\/comments$/.test(path)) {
  if (!a.includes("--paginate")) { process.stderr.write("only the first page\\n"); process.exit(9); }
  for (const c of st.comments || []) console.log(JSON.stringify(c));
  process.exit(0);
}
process.exit(2);
`;

const PATH0 = process.env.PATH;

function makeWorld(): World {
  const tmp = mkdtempSync(join(tmpdir(), "rubato-wait-"));
  const bare = join(tmp, "acme", "map.git");
  mkdirSync(bare, { recursive: true });
  execFileSync("git", ["init", "-q", "--bare", "-b", "main", bare]);
  const work = join(tmp, "work");
  mkdirSync(work);
  git(work, "init", "-q", "-b", "main");
  git(work, "remote", "add", "origin", bare);
  writeFileSync(join(work, "system-map.yml"), readFileSync(join(root, "docs/design/rubato-map.yml"), "utf8"));
  mkdirSync(join(work, "docs/decisions"), { recursive: true });
  mkdirSync(join(work, "docs/plans"), { recursive: true });
  const adr = "docs/decisions/0099-try-it.md";
  writeFileSync(join(work, adr), "# 0099. Try it\n\n- **Status:** proposed\n- **Date:** 2026-11-12\n- **Decided by:** agent (test); a human accepts\n- **Seam:** none\n- **Issue:** acme/map#7\n\n## Decision\n\nTry it.\n");
  const other = "docs/decisions/0098-other.md";
  writeFileSync(join(work, other), "# 0098. Other\n\n- **Status:** proposed\n- **Decided by:** agent\n- **Seam:** none\n- **Issue:** acme/map#8\n\n## Decision\n\nNot ours.\n");
  const plan = "docs/plans/7.md";
  const valid = readFileSync(join(root, "schema/fixtures/plans/valid.md"), "utf8").replace("issue: m0x6A/rubato#42", "issue: acme/map#7");
  writeFileSync(join(work, plan), `${valid}\n## ADRs proposed\n\n| ADR | File | Blob |\n| --- | --- | --- |\n| 0099 | ${adr} | x |\n`);
  git(work, "add", "-A");
  git(work, "commit", "-q", "-m", "map");
  git(work, "push", "-q", "origin", "main");
  git(work, "checkout", "-q", "-b", "issue/7-try");
  const mapClone = { repo: "acme/map", path: "system-map.yml", local: work };
  const ws: Workspace = { map: mapClone, maps: [mapClone], repos: [] };
  const bin = join(tmp, "bin");
  mkdirSync(bin);
  writeFileSync(join(bin, "gh"), GH);
  chmodSync(join(bin, "gh"), 0o755);
  const stubFile = join(tmp, "gh.json");
  process.env.PATH = `${bin}:${PATH0}`;
  process.env.GH_STUB = stubFile;
  const stub = (s: Stub) => writeFileSync(stubFile, JSON.stringify({ issue_url: "https://api.github.com/repos/acme/map/issues/7", ...s }));
  stub({});
  return { work, ws, plan, adr, other, sha: "", stub };
}

async function pinAndPush(w: World): Promise<void> {
  await pinAdrs(w.work, w.plan);
  git(w.work, "add", "-A");
  git(w.work, "commit", "-q", "-m", "plan");
  git(w.work, "push", "-q", "-u", "origin", "issue/7-try");
  w.sha = planSha(readFileSync(join(w.work, w.plan), "utf8"));
}

function comment(id: number, body: string, extra: Partial<GhComment> = {}): GhComment {
  const at = `2026-11-12T10:0${id}:00Z`;
  return { id, body, created_at: at, updated_at: at, html_url: `https://github.com/acme/map/issues/7#issuecomment-${id}`, author_association: "MEMBER", user: { login: "teammate", type: "User" }, ...extra };
}

async function runWait(w: World, o: Parameters<typeof wait>[3] = {}): Promise<{ code: number; lines: Record<string, unknown>[] }> {
  const lines: Record<string, unknown>[] = [];
  const code = await wait(w.ws, w.work, w.plan, { once: true, out: (l) => lines.push(JSON.parse(l) as Record<string, unknown>), ...o });
  return { code, lines };
}

describe("plan.ts wait (ADR 0035)", () => {
  let w: World;
  beforeEach(async () => {
    w = makeWorld();
    await pinAndPush(w);
  });
  afterEach(() => {
    process.env.PATH = PATH0;
  });

  it("returns approved for the newest trusted approval of the pushed sha, a prefix too, reading every page", async () => {
    w.stub({ comments: [comment(1, `Approved: plan ${w.sha}`)] });
    const r = await runWait(w);
    expect(r.code).toBe(WAIT_EXIT.approved);
    expect(r.lines[0]).toMatchObject({ event: "rubato.plan.wait", kind: "started", sha: w.sha, "rubato.issue": "acme/map#7" });
    expect(r.lines.at(-1)).toMatchObject({ kind: "approved", author: "teammate", self: false });
    w.stub({ login: "teammate", comments: [comment(1, `Approved: plan ${w.sha.slice(0, 7)}`)] });
    expect((await runWait(w)).lines.at(-1)).toMatchObject({ kind: "approved", self: true });
  });

  it("reports what does not count and keeps waiting", async () => {
    w.stub({
      comments: [
        comment(1, "Review: full — passed"),
        comment(2, "Decision: go"),
        comment(3, "Approved: plan deadbeef1"),
        comment(4, `Approved: plan ${w.sha}`, { user: { login: "x[bot]", type: "Bot" }, author_association: "OWNER" }),
        comment(5, `Approved: plan ${w.sha}`, { author_association: "CONTRIBUTOR" }),
        comment(6, `Approved: plan ${w.sha}`, { updated_at: "2026-11-12T12:00:00Z" }),
      ],
    });
    const r = await runWait(w);
    expect(r.code).toBe(WAIT_EXIT.not_yet);
    expect(r.lines.slice(1).map((l) => l.kind)).toEqual(["review", "decision", "approved_earlier", "untrusted", "untrusted", "edited"]);
  });

  it("a newer approval of another sha after the current one is not an approval", async () => {
    w.stub({ comments: [comment(1, `Approved: plan ${w.sha}`), comment(2, "Approved: plan deadbeef1")] });
    expect((await runWait(w)).code).toBe(WAIT_EXIT.not_yet);
  });

  it("exits with a reason instead of waiting on what it cannot see", async () => {
    writeFileSync(join(w.work, w.plan), `${readFileSync(join(w.work, w.plan), "utf8")}\nedit\n`);
    let r = await runWait(w);
    expect([r.code, r.lines.at(-1)]).toEqual([WAIT_EXIT.local_edit, expect.objectContaining({ kind: "exited", reason: "local_edit" })]);
    git(w.work, "checkout", "-q", "--", w.plan);

    git(w.work, "checkout", "-q", "-b", "issue/7-unpushed");
    r = await runWait(w);
    expect(r.lines.at(-1)).toMatchObject({ kind: "exited", reason: "not_pushed" });
    git(w.work, "checkout", "-q", "issue/7-try");

    w.stub({ fail: true });
    r = await runWait(w);
    expect(r.lines.map((l) => l.kind)).toEqual(["started", "poll_failed", "exited"]);
    expect(r.lines[1]).toMatchObject({ message: "gh: HTTP 502", code: 1 });
    r = await runWait(w, { once: false, maxFailures: 2, sleep: async () => {} });
    expect([r.code, r.lines.filter((l) => l.kind === "poll_failed").length]).toEqual([WAIT_EXIT.gh_failed, 2]);
  });

  it("exits does_not_fit for a plan naming an unknown part, and plan_changed when the branch moves", async () => {
    w.stub({ comments: [] });
    const r1 = await runWait(w, {
      once: false,
      sleep: async () => {
        writeFileSync(join(w.work, w.plan), `${readFileSync(join(w.work, w.plan), "utf8")}\nmore\n`);
        git(w.work, "commit", "-q", "-am", "edit");
        git(w.work, "push", "-q");
      },
    });
    expect(r1.lines.at(-1)).toMatchObject({ kind: "exited", reason: "plan_changed" });
    writeFileSync(join(w.work, w.plan), readFileSync(join(w.work, w.plan), "utf8").replace("| rubato-server |", "| no-such-part |"));
    git(w.work, "commit", "-q", "-am", "bad");
    git(w.work, "push", "-q");
    const r2 = await runWait(w);
    expect([r2.code, r2.lines.at(-1)?.reason]).toEqual([WAIT_EXIT.does_not_fit, "does_not_fit"]);
  });
});

describe("adrs.ts accept (ADR 0035)", () => {
  let w: World;
  beforeEach(async () => {
    w = makeWorld();
    await pinAndPush(w);
  });
  afterEach(() => {
    process.env.PATH = PATH0;
  });

  it("pins the ADR blob in the plan", () => {
    const rows = pinnedAdrs(readFileSync(join(w.work, w.plan), "utf8"));
    expect(rows).toEqual([{ adr: "0099", file: w.adr, blob: gitBlobSha(readFileSync(join(w.work, w.adr))) }]);
  });

  it("accepts exactly the pinned ADR, citing the comment, and touches nothing else", async () => {
    const before = readFileSync(join(w.work, w.adr), "utf8");
    const otherBefore = readFileSync(join(w.work, w.other));
    w.stub({ login: "teammate", comments: [comment(1, `Approved: plan ${w.sha}`)] });
    const r = await accept(w.ws, w.work, "https://github.com/acme/map/issues/7#issuecomment-1", w.plan, "2026-11-12");
    expect(r.code).toBe(0);
    expect(r.line).toMatchObject({ event: "rubato.adrs.accept", accepted: ["0099"], refused: [], self: true, unpinned: [] });
    const after = readFileSync(join(w.work, w.adr), "utf8");
    expect(after).toContain("- **Status:** accepted\n");
    expect(after).toContain("- **Decided by:** teammate, accepted 2026-11-12 by approval comment https://github.com/acme/map/issues/7#issuecomment-1 (self); drafted by agent (test)\n");
    const rest = (t: string) => t.split("\n").filter((l) => !/^- \*\*(Status|Decided by):/.test(l));
    expect(rest(after)).toEqual(rest(before));
    expect(readFileSync(join(w.work, w.other)).equals(otherBefore)).toBe(true);
    // Running it again accepts nothing more and writes nothing.
    const again = await accept(w.ws, w.work, "1", w.plan, "2026-11-12");
    expect(again.line).toMatchObject({ accepted: [], refused: [{ adr: "0099", reason: "adr_changed" }] });
    expect(readFileSync(join(w.work, w.adr), "utf8")).toBe(after);
  });

  it("refuses an ADR changed after it was pinned, and reports an unpinned one", async () => {
    writeFileSync(join(w.work, w.adr), `${readFileSync(join(w.work, w.adr), "utf8")}\nOne more line.\n`);
    writeFileSync(join(w.work, "docs/decisions/0097-late.md"), "# 0097. Late\n\n- **Status:** proposed\n- **Issue:** acme/map#7\n");
    const changed = readFileSync(join(w.work, w.adr), "utf8");
    w.stub({ comments: [comment(1, `Approved: plan ${w.sha}`)] });
    const r = await accept(w.ws, w.work, "1", w.plan);
    expect(r.code).toBe(1);
    expect(r.line).toMatchObject({ accepted: [], refused: [{ adr: "0099", reason: "adr_changed" }], unpinned: ["0097-late.md"] });
    expect(readFileSync(join(w.work, w.adr), "utf8")).toBe(changed);
  });

  it("refuses, writing nothing, when the approval does not count or this is not the map repo", async () => {
    const before = readFileSync(join(w.work, w.adr), "utf8");
    const cases: [Stub, string, string][] = [
      [{ comments: [comment(1, "Approved: plan deadbeef1")] }, "1", "earlier_sha"],
      [{ comments: [comment(1, `Approved: plan ${w.sha}`, { updated_at: "2026-11-12T12:00:00Z" })] }, "1", "edited"],
      [{ comments: [comment(1, `Approved: plan ${w.sha}`, { author_association: "CONTRIBUTOR" })] }, "1", "untrusted"],
      [{ comments: [comment(1, `Approved: plan ${w.sha}`), comment(2, `Approved: plan ${w.sha}`)] }, "1", "not_newest"],
      [{ comments: [comment(1, `Approved: plan ${w.sha}`)], issue_url: "https://api.github.com/repos/acme/map/issues/8" }, "1", "wrong_issue"],
    ];
    for (const [s, id, reason] of cases) {
      w.stub(s);
      const r = await accept(w.ws, w.work, id, w.plan);
      expect([r.code, r.line.refused], reason).toEqual([1, reason]);
    }
    w.stub({ comments: [comment(1, `Approved: plan ${w.sha}`)] });
    const other = { ...w.ws.map, repo: "acme/other" };
    const elsewhere = await accept({ ...w.ws, map: other, maps: [other] }, w.work, "1", w.plan);
    expect(elsewhere.line).toMatchObject({ refused: "not_map_repo", would_accept: ["0099"] });
    expect(readFileSync(join(w.work, w.adr), "utf8")).toBe(before);
  });
});

describe("no comment is posted by wait or accept (ADR 0027)", () => {
  it("the scripts call gh only to read", () => {
    const dir = join(root, "skills/rubato/scripts");
    const sources = ["plan.ts", "adrs.ts", "lib/approval.ts", "lib/comments.ts"].map((f) => readFileSync(join(dir, f), "utf8"));
    for (const s of sources) {
      expect(s).not.toMatch(/issue",\s*"comment"|"--method",\s*"POST"|-X\s*POST|"comment",/);
    }
    // Fails on purpose: the same scan finds a posting call.
    expect(/issue",\s*"comment"/.test(`gh(["issue", "comment", "7"])`)).toBe(true);
  });
});

describe("install ships the wait (ADR 0035)", () => {
  afterEach(() => {
    process.env.PATH = PATH0;
  });

  it("copies every shipped file byte for byte, and the installed copy runs wait --once", async () => {
    const skill = join(root, "skills/rubato");
    const to = join(mkdtempSync(join(tmpdir(), "rubato-install-")), "rubato");
    install(to, false);
    const shipped = (d: string): string[] =>
      readdirSync(join(skill, d), { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? shipped(join(d, e.name)) : [join(d, e.name)]));
    const files = ["SKILL.md", "hooks.json", "prompt.md", "decision-template.md", "review-template.md", "review-checklist.md", "local.example.yml", ...shipped("scripts")];
    for (const f of files) {
      expect(readFileSync(join(to, f)).equals(readFileSync(join(skill, f))), f).toBe(true);
    }
    expect(files).toContain(join("scripts", "lib", "comments.ts"));
    symlinkSync(join(skill, "node_modules"), join(to, "node_modules"));

    const w = makeWorld();
    await pinAndPush(w);
    w.stub({ comments: [comment(1, `Approved: plan ${w.sha}`)] });
    const yml = join(w.work, "..", "workspace.yml");
    writeFileSync(yml, `map:\n  repo: acme/map\n  path: system-map.yml\n  local: ${w.work}\nrepos: []\n`);
    const out = execFileSync("node", ["--experimental-strip-types", join(to, "scripts/plan.ts"), "wait", "--once", w.plan], {
      cwd: w.work,
      encoding: "utf8",
      env: { ...process.env, RUBATO_WORKSPACE: yml },
      stdio: ["ignore", "pipe", "pipe"],
    });
    const last = JSON.parse(out.trim().split("\n").at(-1) as string) as { kind: string };
    expect(last.kind).toBe("approved");
    console.log(`checked ${files.length} shipped files and one installed wait`);
  });
});
