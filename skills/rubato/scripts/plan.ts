#!/usr/bin/env -S node --experimental-strip-types
/**
 * Create and check the plan file at the agreed path (ADR 0005, ADR 0009, ADR 0027), and print
 * the version an approval names.
 *
 *   plan.ts init  --issue <owner/name#n> --slug <slug> [--title <t>] [--cwd <repo>] [--spend-cap <usd>] [--attempts <n>]
 *   plan.ts path  --issue <owner/name#n> --slug <slug> [--cwd <repo>]
 *   plan.ts check [<plan path>] [--cwd <repo>]
 *   plan.ts sha   [<plan path>] [--cwd <repo>]
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { currentBranch, issueFromBranch, originRepo } from "./lib/git.ts";
import { planSha } from "./lib/plan-sha.ts";
import { planCheck, schemaDir } from "./lib/vendor.ts";
import { loadWorkspace, matchedMaps, repoForCwd, type Workspace } from "./lib/local.ts";

export interface InitOptions {
  issue: string;
  slug: string;
  title?: string;
  spendCapUsd?: number;
  attemptLimit?: number;
}

/**
 * The only front-matter keys a plan carries (ADR 0027). Track, review and approval are issue
 * comments that Rubato reads; status is derived, never written.
 */
export const FRONT_MATTER_KEYS = ["issue", "title", "map_version", "spend_cap_usd", "attempt_limit"] as const;

/** The spec headings, the same as the in-app issue form's (ADR 0028). */
export const SPEC_HEADINGS = ["What the change does", "How we'll know it works", "What it must not break", "How we'll debug it at 3 am"] as const;

export function planTemplate(o: InitOptions, mapVersion: string | null): string {
  const title = o.title ?? `Issue ${o.issue}`;
  return `---
issue: ${o.issue}
title: ${yamlScalar(title)}
${mapVersion === null ? "" : `map_version: ${mapVersion}\n`}spend_cap_usd: ${o.spendCapUsd ?? 25}
attempt_limit: ${o.attemptLimit ?? 3}
---

# ${title}

One paragraph: what this change does and why, in the words of the issue.

## ${SPEC_HEADINGS[0]}

<!-- Behaviour, one bullet each, in the user's or the other side's terms. -->

-

## ${SPEC_HEADINGS[1]}

<!-- Checkboxes, each one a test or an observation that proves a bullet above. -->

- [ ]

## ${SPEC_HEADINGS[2]}

<!-- What keeps working, and what guards it (an existing test, a check, a scenario). -->

-

## ${SPEC_HEADINGS[3]}

<!-- Point at the O-rows below; name any log line or finding that is new. -->

See the O-rows.

## Affected parts

<!-- One row per map node this change touches. Status: affected | new | quiet. Seams: comma-separated seam ids. -->

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| | affected | | |

## Data models

<!-- Every table, wire type, schema, event or file this change introduces, changes or removes, on the part that owns it (ADR 0031). Kind: table | type | schema | event | file. Change: new | changed | removed. Crosses: the seam it travels over, if any. Write "None." instead of the table if there are none. -->

| Model | Kind | Part | Change | Crosses | Note |
| --- | --- | --- | --- | --- | --- |
| | | | | | |

## Data flow

<!-- Mermaid: the data crossing each seam in the table above. -->

## Tasks

<!-- Ordered; each names its failing test first. -->

1.

## Assumed, not yet approved

<!-- Decisions nothing settled. Scope: node (assume and proceed) | seam (blocks until a human answers). -->

| # | Decision | Chosen | Alternatives | Scope |
| --- | --- | --- | --- | --- |
| A1 | | | | node |

## O-rows

<!-- One per seam in Affected parts: if it breaks at 3 am, which signal leads back, on which panel, who is woken. -->

| Seam | Breaks like | Signal | Panel | Owner |
| --- | --- | --- | --- | --- |
| | | | | |

## Flows

<!-- What this change sends across seams, one row per thing and direction. Kind: data | control. The seam must join From and To. Rubato draws these on the shared map. -->

| From | To | Seam | Kind | Carries |
| --- | --- | --- | --- | --- |
| | | | | |

## ADRs proposed

<!-- Each ADR this plan proposes, pinned by its file's git blob sha so the approval covers its text (ADR 0035). Fill Blob with: plan.ts adrs-pin. Write "None." when there are none. -->

None.

## Review map

1.
`;
}

/** A YAML scalar that reads back as `s`: quoted when it would not. */
export function yamlScalar(s: string): string {
  return /^[\w][\w .,()/-]*$/.test(s) && !/: |\s#/.test(s) ? s : JSON.stringify(s);
}

/** The maps this session plans against: every listed map that names the repo (ADR 0045). */
export async function sessionMaps(ws: Workspace, cwd: string) {
  const { map: mapMod } = await planCheck();
  const repo = repoForCwd(ws, cwd)?.name ?? originRepo(cwd);
  const matched = matchedMaps(ws, repo, (file) => mapMod.loadMap(file, schemaDir()));
  if (matched.length === 0) {
    throw new Error(`no map in local.yml names ${repo ?? cwd}; listed: ${ws.maps.map((m) => m.repo).join(", ")}`);
  }
  return { repo, matched };
}

async function resolvePlanPath(ws: Workspace, cwd: string, issueNumber: number, slug: string): Promise<{ repo: string; path: string }> {
  const { repo, matched } = await sessionMaps(ws, cwd);
  if (!repo) {
    throw new Error(`${cwd} is not a connected repo and has no origin`);
  }
  const templates = [...new Set(matched.map((m) => m.map.repos?.find((r) => r.name === repo)?.plan_path ?? "docs/plans/{issue}.md"))];
  if (templates.length > 1) {
    // The server takes the first workspace's; the skill refuses rather than guess (ADR 0045).
    throw new Error(`the maps for ${repo} give different plan paths (${templates.join(", ")}); make them agree`);
  }
  const template = templates[0] as string;
  return { repo, path: template.replace("{issue}", String(issueNumber)).replace("{slug}", slug) };
}

function parseIssue(s: string): { repo: string; number: number } {
  const m = /^([^\s/#]+\/[^\s/#]+)#(\d+)$/.exec(s);
  if (!m) {
    throw new Error(`--issue must be owner/name#number, got ${s}`);
  }
  return { repo: m[1] as string, number: Number(m[2]) };
}

export async function init(ws: Workspace, cwd: string, o: InitOptions): Promise<string> {
  const issue = parseIssue(o.issue);
  const { path } = await resolvePlanPath(ws, cwd, issue.number, o.slug);
  const abs = resolve(cwd, path);
  if (existsSync(abs)) {
    throw new Error(`${path} already exists; edit it or run plan.ts check`);
  }
  const { headSha } = await import("./lib/git.ts");
  const { matched } = await sessionMaps(ws, cwd);
  // One map: its version. Several: none, which ADR 0027 allows; the plan fits whichever it fits.
  const version = matched.length === 1 ? headSha((matched[0] as (typeof matched)[number]).clone.local) : null;
  console.error(JSON.stringify({ event: "rubato.plan.init", map_version: version === null ? "omitted_several_maps" : "written" }));
  mkdirSync(dirname(abs), { recursive: true });
  writeFileSync(abs, planTemplate(o, version));
  return path;
}

export async function check(ws: Workspace, cwd: string, planPath?: string): Promise<{ ok: boolean; line: Record<string, unknown> }> {
  let path = planPath;
  if (!path) {
    const n = issueFromBranch(currentBranch(cwd));
    if (n === null) {
      throw new Error("not on an issue/<n>-<slug> branch; pass the plan path");
    }
    const r = await resolvePlanPath(ws, cwd, n, "");
    path = r.path;
  }
  const abs = resolve(cwd, path);
  const rel = relative(cwd, abs);
  return fitsAny(ws, cwd, readFileSync(abs, "utf8"), rel);
}

/** The sha a person's `Approved: plan <sha>` comment names for the plan at `planPath`. */
export async function sha(ws: Workspace, cwd: string, planPath?: string): Promise<{ path: string; sha: string }> {
  let path = planPath;
  if (!path) {
    const n = issueFromBranch(currentBranch(cwd));
    if (n === null) {
      throw new Error("not on an issue/<n>-<slug> branch; pass the plan path");
    }
    path = (await resolvePlanPath(ws, cwd, n, "")).path;
  }
  return { path, sha: planSha(readFileSync(resolve(cwd, path), "utf8")) };
}

/** The plan path for the issue the current branch names, as the map places it. */
export async function planPathFromBranch(ws: Workspace, cwd: string): Promise<string> {
  const n = issueFromBranch(currentBranch(cwd));
  if (n === null) {
    throw new Error("not on an issue/<n>-<slug> branch; pass the plan path");
  }
  return (await resolvePlanPath(ws, cwd, n, "")).path;
}

/** Whether the plan text fits the map (ADR 0029), with the `rubato.plan.check` line. */
export async function fits(ws: Workspace, text: string, rel: string, cwd: string = process.cwd()): Promise<{ ok: boolean; line: Record<string, unknown> }> {
  return fitsAny(ws, cwd, text, rel);
}

/**
 * The plan fits when it fits any map the session plans against: the server's rule (ADR 0045).
 * The line is that map's; with several maps it also says how many were checked and which fit.
 */
async function fitsAny(ws: Workspace, cwd: string, text: string, rel: string): Promise<{ ok: boolean; line: Record<string, unknown> }> {
  const { check: checkMod } = await planCheck();
  const { matched } = await sessionMaps(ws, cwd);
  const results = matched.map((m) => ({ m, result: checkMod.checkPlanText(text, rel, m.map, schemaDir()) }));
  const first = results.find((x) => x.result.ok) ?? (results[0] as (typeof results)[number]);
  const line: Record<string, unknown> = checkMod.logLine(first.result);
  if (results.length > 1) {
    line["maps_checked"] = results.length;
    line["fits"] = results.filter((x) => x.result.ok).map((x) => x.m.clone.repo);
  }
  return { ok: first.result.ok, line };
}

/** Exit codes of `plan.ts wait`: one per reason, so a caller can act on it. */
export const WAIT_EXIT = { approved: 0, not_yet: 3, not_pushed: 4, local_edit: 4, does_not_fit: 5, gh_failed: 6, plan_changed: 7 } as const;

export interface WaitOptions {
  intervalSecs?: number;
  once?: boolean;
  /** Consecutive `gh` failures before giving up. */
  maxFailures?: number;
  out?: (line: string) => void;
  sleep?: (ms: number) => Promise<void>;
}

/**
 * Wait for a person's approval of this plan (ADR 0035): the newest trusted, unedited
 * `Approved:` comment naming the sha of the plan as pushed, with the plan fitting the map. Prints
 * one `rubato.plan.wait` line per event, `started` first and `exited` or `approved` last; never
 * silent. Posts nothing.
 */
export async function wait(ws: Workspace, cwd: string, planPath: string | undefined, o: WaitOptions = {}): Promise<number> {
  const out = o.out ?? ((l: string) => console.log(l));
  const sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const { line, planIssue, pushedPlan, issueComments, ghLogin, ghFailure } = await import("./lib/approval.ts");
  const { classify } = await import("./lib/comments.ts");
  const emit = (kind: string, fields: Record<string, unknown> = {}) => out(line("rubato.plan.wait", { kind, ...fields }));
  const path = planPath ?? (await planPathFromBranch(ws, cwd));
  const rel = relative(cwd, resolve(cwd, path));
  const exit = (reason: keyof typeof WAIT_EXIT, fields: Record<string, unknown> = {}) => {
    emit("exited", { reason, ...fields });
    return WAIT_EXIT[reason];
  };
  const read = () => pushedPlan(cwd, rel, readFileSync(resolve(cwd, rel), "utf8"));
  const pushed = read();
  if (!pushed.ok) {
    return exit(pushed.reason, { branch: pushed.branch });
  }
  const issue = planIssue(pushed.text);
  if (!issue) {
    throw new Error(`${rel} has no issue: owner/name#n in its front matter`);
  }
  const ref = `${issue.repo}#${issue.number}`;
  const sha = planSha(pushed.text);
  const fit = await fits(ws, pushed.text, rel, cwd);
  if (!fit.ok) {
    return exit("does_not_fit", { "rubato.issue": ref, sha, check: fit.line });
  }
  let me: string | null = null;
  try {
    me = ghLogin();
  } catch {
    // Without a login, `self` is unknown; the wait still works.
  }
  const interval = o.intervalSecs ?? 30;
  emit("started", { "rubato.issue": ref, sha, branch: pushed.branch, interval });
  const seen = new Set<number>();
  let failures = 0;
  for (;;) {
    let comments;
    try {
      comments = issueComments(issue.repo, issue.number);
      failures = 0;
    } catch (e) {
      failures++;
      emit("poll_failed", { "rubato.issue": ref, ...ghFailure(e), failures });
      if (failures >= (o.maxFailures ?? 5)) {
        return exit("gh_failed", { "rubato.issue": ref });
      }
      if (o.once) {
        return exit("gh_failed", { "rubato.issue": ref });
      }
      await sleep(interval * 1000);
      continue;
    }
    comments.sort((a, b) => (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : a.id - b.id));
    let approvedBy = null;
    for (const c of comments) {
      if (seen.has(c.id)) {
        continue;
      }
      seen.add(c.id);
      const kind = classify(c, sha);
      if (kind && kind !== "approved") {
        emit(kind, { "rubato.issue": ref, author: c.user.login, url: c.html_url });
      }
    }
    const { approvalFor } = await import("./lib/comments.ts");
    const verdict = approvalFor(comments, sha);
    if (verdict.ok) {
      approvedBy = verdict.comment;
    }
    if (approvedBy) {
      emit("approved", { "rubato.issue": ref, sha, author: approvedBy.user.login, self: me !== null && approvedBy.user.login === me, url: approvedBy.html_url });
      return WAIT_EXIT.approved;
    }
    if (o.once) {
      return WAIT_EXIT.not_yet;
    }
    await sleep(interval * 1000);
    const again = read();
    if (!again.ok) {
      return exit(again.reason, { branch: again.branch });
    }
    if (planSha(again.text) !== sha) {
      return exit("plan_changed", { "rubato.issue": ref, sha, now: planSha(again.text) });
    }
  }
}

function opt(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
}

async function main(argv: string[]): Promise<number> {
  const [cmd, ...rest] = argv;
  const cwd = resolve(opt(rest, "--cwd") ?? process.cwd());
  try {
    const given = rest.filter((a, i) => !a.startsWith("--") && rest[i - 1] !== "--cwd")[0];
    if (cmd === "sha" && given) {
      // A named file needs no workspace: the hash depends on the text alone.
      console.log(planSha(readFileSync(resolve(cwd, given), "utf8")));
      return 0;
    }
    const ws = loadWorkspace();
    if (cmd === "init" || cmd === "path") {
      const issue = opt(rest, "--issue");
      const slug = opt(rest, "--slug");
      if (!issue || !slug) {
        throw new Error("--issue and --slug are required");
      }
      if (cmd === "path") {
        const r = await resolvePlanPath(ws, cwd, parseIssue(issue).number, slug);
        console.log(r.path);
        return 0;
      }
      const o: InitOptions = { issue, slug };
      const title = opt(rest, "--title");
      if (title) {
        o.title = title;
      }
      const cap = opt(rest, "--spend-cap");
      if (cap) {
        o.spendCapUsd = Number(cap);
      }
      const attempts = opt(rest, "--attempts");
      if (attempts) {
        o.attemptLimit = Number(attempts);
      }
      const path = await init(ws, cwd, o);
      console.log(`wrote ${path}; fill the tables, then: plan.ts check ${path}`);
      return 0;
    }
    if (cmd === "adrs-pin") {
      const positional = rest.filter((a, i) => !a.startsWith("--") && rest[i - 1] !== "--cwd");
      const { pinAdrs } = await import("./adrs.ts");
      const rows = await pinAdrs(cwd, positional[0] ?? (await planPathFromBranch(ws, cwd)));
      console.log(JSON.stringify({ pinned: rows }));
      return 0;
    }
    if (cmd === "wait") {
      const positional = rest.filter((a, i) => !a.startsWith("--") && rest[i - 1] !== "--cwd" && rest[i - 1] !== "--interval");
      const o: WaitOptions = { once: rest.includes("--once") };
      const interval = opt(rest, "--interval");
      if (interval) {
        o.intervalSecs = Number(interval);
      }
      return await wait(ws, cwd, positional[0], o);
    }
    if (cmd === "check" || cmd === "sha") {
      const positional = rest.filter((a, i) => !a.startsWith("--") && rest[i - 1] !== "--cwd");
      if (cmd === "sha") {
        const r = await sha(ws, cwd, positional[0]);
        console.log(r.sha);
        return 0;
      }
      const r = await check(ws, cwd, positional[0]);
      console.log(JSON.stringify(r.line));
      return r.ok ? 0 : 1;
    }
    console.error("usage: plan.ts init|path|check|sha|wait|adrs-pin ...");
    return 2;
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = await main(process.argv.slice(2));
}

export { join };
