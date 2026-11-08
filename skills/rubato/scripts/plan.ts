#!/usr/bin/env -S node --experimental-strip-types
/**
 * Create and check the plan file at the agreed path (ADR 0005, ADR 0009).
 *
 *   plan.ts init  --issue <owner/name#n> --slug <slug> [--title <t>] [--cwd <repo>] [--spend-cap <usd>] [--attempts <n>]
 *   plan.ts path  --issue <owner/name#n> --slug <slug> [--cwd <repo>]
 *   plan.ts check [<plan path>] [--cwd <repo>]
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { currentBranch, issueFromBranch, originRepo } from "./lib/git.ts";
import { planCheck, schemaDir } from "./lib/vendor.ts";
import { loadWorkspace, mapFile, repoForCwd, type Workspace } from "./lib/workspace.ts";

export interface InitOptions {
  issue: string;
  slug: string;
  title?: string;
  spendCapUsd?: number;
  attemptLimit?: number;
}

export function planTemplate(o: InitOptions, mapVersion: string): string {
  const title = o.title ?? `Issue ${o.issue}`;
  return `---
issue: ${o.issue}
title: ${title}
status: proposed
date: ${new Date().toISOString().slice(0, 10)}
map_version: ${mapVersion}
spend_cap_usd: ${o.spendCapUsd ?? 25}
attempt_limit: ${o.attemptLimit ?? 3}
---

# ${title}

One paragraph: what this change does and why, in the words of the issue.

## Affected parts

<!-- One row per map node this change touches. Status: affected | new | quiet. Seams: comma-separated seam ids. -->

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| | affected | | |

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

## Review map

1.
`;
}

async function resolvePlanPath(ws: Workspace, cwd: string, issueNumber: number, slug: string): Promise<{ repo: string; path: string }> {
  const { map: mapMod } = await planCheck();
  const map = mapMod.loadMap(mapFile(ws), schemaDir());
  const repo = repoForCwd(ws, cwd)?.name ?? originRepo(cwd);
  if (!repo) {
    throw new Error(`${cwd} is not a connected repo and has no origin`);
  }
  const template = map.repos?.find((r) => r.name === repo)?.plan_path ?? "docs/plans/{issue}.md";
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
  mkdirSync(dirname(abs), { recursive: true });
  writeFileSync(abs, planTemplate(o, headSha(ws.map.local)));
  return path;
}

export async function check(ws: Workspace, cwd: string, planPath?: string): Promise<{ ok: boolean; line: Record<string, unknown> }> {
  const { check: checkMod, map: mapMod } = await planCheck();
  const map = mapMod.loadMap(mapFile(ws), schemaDir());
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
  const result = checkMod.checkPlanText(readFileSync(abs, "utf8"), rel, map, schemaDir());
  return { ok: result.ok, line: checkMod.logLine(result) };
}

function opt(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
}

async function main(argv: string[]): Promise<number> {
  const [cmd, ...rest] = argv;
  const cwd = resolve(opt(rest, "--cwd") ?? process.cwd());
  try {
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
    if (cmd === "check") {
      const positional = rest.filter((a, i) => !a.startsWith("--") && rest[i - 1] !== "--cwd");
      const r = await check(ws, cwd, positional[0]);
      console.log(JSON.stringify(r.line));
      return r.ok ? 0 : 1;
    }
    console.error("usage: plan.ts init|path|check ...");
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
