#!/usr/bin/env -S node --experimental-strip-types
/**
 * List the accepted ADRs from the map repo so a plan does not contradict one.
 *
 *   adrs.ts [--all] [--json]
 *   adrs.ts accept --comment <url> [--plan <path>] [--cwd <dir>]
 *
 * `accept` records that a person's approval of the plan accepted the ADRs it pins (ADR 0035).
 */
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { loadWorkspace, type Workspace } from "./lib/workspace.ts";

export interface Adr {
  file: string;
  number: string;
  title: string;
  status: string;
  seam: string;
  decision: string;
}

function field(text: string, name: string): string {
  const m = new RegExp(`^- \\*\\*${name}:\\*\\*\\s*(.+)$`, "m").exec(text);
  return m ? (m[1] as string).trim() : "";
}

function section(text: string, heading: string): string {
  const m = new RegExp(`^## ${heading}\\s*\\n([\\s\\S]*?)(?=^## |\\Z)`, "m").exec(text);
  return m ? (m[1] as string).trim() : "";
}

export function parseAdr(file: string, text: string): Adr | null {
  const title = /^# (\d{4})\.\s*(.+)$/m.exec(text);
  if (!title) {
    return null;
  }
  return {
    file,
    number: title[1] as string,
    title: (title[2] as string).trim(),
    status: field(text, "Status").split(/\s+/)[0] ?? "",
    seam: field(text, "Seam"),
    decision: section(text, "Decision"),
  };
}

export function listAdrs(ws: Workspace): Adr[] {
  const dir = join(ws.map.local, ws.map.decisions ?? "docs/decisions");
  const out: Adr[] = [];
  for (const f of readdirSync(dir).sort()) {
    if (!/^\d{4}-.*\.md$/.test(f) || f.startsWith("0000-")) {
      continue;
    }
    const adr = parseAdr(f, readFileSync(join(dir, f), "utf8"));
    if (adr) {
      out.push(adr);
    }
  }
  return out;
}

export function render(adrs: Adr[]): string {
  return adrs
    .map((a) => `## ${a.number}. ${a.title}\n- status: ${a.status}; seam: ${a.seam}; file: ${a.file}\n\n${a.decision}`)
    .join("\n\n");
}

/** One row of a plan's `## ADRs proposed` table. */
export interface PinnedAdr {
  adr: string;
  file: string;
  blob: string;
}

/** The `## ADRs proposed` table of a plan; empty when the section is absent or says None. */
export function pinnedAdrs(plan: string): PinnedAdr[] {
  const m = /^## ADRs proposed\s*\n([\s\S]*?)(?=^## |(?![\s\S]))/m.exec(plan);
  if (!m) {
    return [];
  }
  return (m[1] as string)
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.startsWith("|") && !/^\|\s*-/.test(l))
    .map((l) => l.slice(1, l.endsWith("|") ? -1 : undefined).split("|").map((c) => c.trim().replace(/^`|`$/g, "")))
    .filter((cells) => cells.length >= 3 && /^\d{4}$/.test(cells[0] as string))
    .map((cells) => ({ adr: cells[0] as string, file: cells[1] as string, blob: (cells[2] as string).toLowerCase() }));
}

/** Fill the Blob column of a plan's `## ADRs proposed` table from the files in `cwd`. */
export async function pinAdrs(cwd: string, planPath: string): Promise<PinnedAdr[]> {
  const { gitBlobSha } = await import("./lib/plan-sha.ts");
  const abs = resolve(cwd, planPath);
  const text = readFileSync(abs, "utf8");
  const rows = pinnedAdrs(text);
  const blobs = new Map(rows.map((r) => [r.adr, gitBlobSha(readFileSync(join(cwd, r.file)))]));
  let inSection = false;
  const lines = text.split("\n").map((l) => {
    if (l.startsWith("## ")) {
      inSection = l.trim() === "## ADRs proposed";
      return l;
    }
    const cells = l.trim().startsWith("|") ? l.trim().slice(1, -1).split("|") : [];
    const blob = inSection && cells.length >= 3 ? blobs.get((cells[0] as string).trim()) : undefined;
    if (!blob) {
      return l;
    }
    cells[2] = ` ${blob} `;
    return `|${cells.join("|")}|`;
  });
  writeFileSync(abs, lines.join("\n"));
  for (const r of rows) {
    r.blob = blobs.get(r.adr) as string;
  }
  return rows;
}

export type AcceptResult = { code: number; line: Record<string, unknown> };

/**
 * Record the acceptance of a plan's pinned ADRs by a person's approval comment (ADR 0035).
 * Every rule is checked again from GitHub before anything is written; any failure writes
 * nothing for the comment, and each ADR that is missing, not proposed or changed since it was
 * pinned is refused on its own. Writes only in `cwd`, and only when `cwd` is the map repo.
 */
export async function accept(ws: Workspace, cwd: string, commentRef: string, planPath?: string, today = new Date().toISOString().slice(0, 10)): Promise<AcceptResult> {
  const { planPathFromBranch, fits } = await import("./plan.ts");
  const { planSha, gitBlobSha } = await import("./lib/plan-sha.ts");
  const { pushedPlan, planIssue, issueComment, issueComments, ghLogin, commentId } = await import("./lib/approval.ts");
  const { approvalFor, isTrusted } = await import("./lib/comments.ts");
  const { originRepo } = await import("./lib/git.ts");
  const done = (code: number, fields: Record<string, unknown>): AcceptResult => ({ code, line: { event: "rubato.adrs.accept", ...fields } });
  const refuse = (reason: string, extra: Record<string, unknown> = {}) => done(1, { refused: reason, accepted: [], ...extra });

  const rel = relative(cwd, resolve(cwd, planPath ?? (await planPathFromBranch(ws, cwd))));
  const pushed = pushedPlan(cwd, rel, readFileSync(resolve(cwd, rel), "utf8"));
  if (!pushed.ok) {
    return refuse(pushed.reason);
  }
  const issue = planIssue(pushed.text);
  if (!issue) {
    return refuse("no_issue");
  }
  const ref = `${issue.repo}#${issue.number}`;
  const sha = planSha(pushed.text);
  const fit = await fits(ws, pushed.text, rel);
  if (!fit.ok) {
    return refuse("does_not_fit", { "rubato.issue": ref });
  }
  const id = commentId(commentRef);
  if (id === null) {
    return refuse("bad_comment");
  }
  const c = issueComment(issue.repo, id);
  if (!c.issue_url.endsWith(`/issues/${issue.number}`)) {
    return refuse("wrong_issue", { "rubato.issue": ref });
  }
  if (!isTrusted(c)) {
    return refuse("untrusted", { "rubato.issue": ref, author: c.user.login });
  }
  const verdict = approvalFor(issueComments(issue.repo, issue.number), sha);
  if (!verdict.ok) {
    return refuse(verdict.reason, { "rubato.issue": ref, sha });
  }
  if (verdict.comment.id !== id) {
    return refuse("not_newest", { "rubato.issue": ref, newest: verdict.comment.html_url });
  }
  const rows = pinnedAdrs(pushed.text);
  if ((originRepo(cwd) ?? "").toLowerCase() !== ws.map.repo.toLowerCase()) {
    return refuse("not_map_repo", { "rubato.issue": ref, would_accept: rows.map((r) => r.adr) });
  }
  let me: string | null = null;
  try {
    me = ghLogin();
  } catch {
    // `self` stays unknown.
  }
  const self = me !== null && me === c.user.login;
  const accepted: string[] = [];
  const refused: { adr: string; reason: string }[] = [];
  const writes: [string, string][] = [];
  for (const r of rows) {
    const abs = join(cwd, r.file);
    if (!existsSync(abs)) {
      refused.push({ adr: r.adr, reason: "missing" });
      continue;
    }
    const text = readFileSync(abs, "utf8");
    if (gitBlobSha(text) !== r.blob) {
      refused.push({ adr: r.adr, reason: "adr_changed" });
      continue;
    }
    if (field(text, "Status").split(/\s+/)[0] !== "proposed") {
      refused.push({ adr: r.adr, reason: "not_proposed" });
      continue;
    }
    const decider = field(text, "Decided by").replace(/;\s*a human accepts\s*$/, "");
    const next = text
      .replace(/^- \*\*Status:\*\*\s*proposed\b.*$/m, "- **Status:** accepted")
      .replace(/^- \*\*Decided by:\*\*.*$/m, `- **Decided by:** ${c.user.login}, accepted ${today} by approval comment ${c.html_url}${self ? " (self)" : ""}; drafted by ${decider}`);
    writes.push([abs, next]);
    accepted.push(r.adr);
  }
  const pinned = new Set(rows.map((r) => r.file));
  const dir = join(cwd, ws.map.decisions ?? "docs/decisions");
  const unpinned = (existsSync(dir) ? readdirSync(dir) : [])
    .filter((f) => /^\d{4}-.*\.md$/.test(f))
    .filter((f) => {
      const t = readFileSync(join(dir, f), "utf8");
      return field(t, "Status").startsWith("proposed") && field(t, "Issue") === ref && !pinned.has(relative(cwd, join(dir, f)));
    });
  for (const [abs, next] of writes) {
    writeFileSync(abs, next);
  }
  return done(refused.length ? 1 : 0, { "rubato.issue": ref, sha, comment: c.html_url, author: c.user.login, self, accepted, refused, unpinned });
}

async function main(argv: string[]): Promise<number> {
  try {
    if (argv[0] === "accept") {
      const at = (n: string) => (argv.indexOf(n) >= 0 ? argv[argv.indexOf(n) + 1] : undefined);
      const comment = at("--comment");
      if (!comment) {
        throw new Error("--comment <url> is required");
      }
      const r = await accept(loadWorkspace(), resolve(at("--cwd") ?? process.cwd()), comment, at("--plan"));
      console.log(JSON.stringify(r.line));
      return r.code;
    }
    let adrs = listAdrs(loadWorkspace());
    if (!argv.includes("--all")) {
      adrs = adrs.filter((a) => a.status === "accepted");
    }
    console.log(argv.includes("--json") ? JSON.stringify(adrs, null, 2) : render(adrs));
    return 0;
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = await main(process.argv.slice(2));
}
