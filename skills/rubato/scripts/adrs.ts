#!/usr/bin/env -S node --experimental-strip-types
/**
 * List the accepted ADRs from the map repo so a plan does not contradict one.
 *
 *   adrs.ts [--all] [--json]
 *   adrs.ts accept --comment <url> [--plan <path>] [--cwd <dir>]
 *
 * `accept` recorded that a person's `Approved:` comment accepted the ADRs a plan pins (ADR 0035). It is
 * retired at the cutover (ADR 0054): it prints `approval_in_rubato` and exits 11, since approval in
 * Rubato accepts them and `plan.ts handoff` records it. `accept()` stays for that history.
 */
import { existsSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";
import { loadWorkspace, type Workspace } from "./lib/local.ts";

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

/** The ADRs of every listed map (ADR 0044): each workspace's map repo keeps its own decisions. */
export function listAdrs(ws: Workspace): Adr[] {
  const out: Adr[] = [];
  const dirs = [...new Set(ws.maps.map((m) => join(m.local, m.decisions ?? "docs/decisions")))];
  for (const dir of dirs) {
  for (const f of readdirSync(dir).sort()) {
    if (!/^\d{4}-.*\.md$/.test(f) || f.startsWith("0000-")) {
      continue;
    }
    const adr = parseAdr(f, readFileSync(join(dir, f), "utf8"));
    if (adr) {
      out.push(adr);
    }
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

/**
 * The ADR file a plan row names, resolved against `cwd`, or null when it lies outside the
 * decisions folder `dir` (relative to `cwd`): a `..`, an absolute path elsewhere, or a symlink
 * that leads out. The plan is text an agent wrote, so its File column is not trusted (#195).
 */
export function decisionFile(cwd: string, dir: string, file: string): string | null {
  const within = (base: string, p: string) => {
    const rel = relative(base, p);
    return rel !== "" && !rel.startsWith("..") && !isAbsolute(rel);
  };
  const base = resolve(cwd, dir);
  const abs = resolve(cwd, file);
  if (!within(base, abs)) {
    return null;
  }
  if (existsSync(abs) && !(existsSync(base) && within(realpathSync(base), realpathSync(abs)))) {
    return null;
  }
  return abs;
}

/**
 * Fill the Blob column of a plan's `## ADRs proposed` table from the files in `cwd`. Throws,
 * writing nothing, when a row names a file outside `decisions` (relative to `cwd`).
 */
export async function pinAdrs(cwd: string, planPath: string, decisions = "docs/decisions"): Promise<PinnedAdr[]> {
  const { gitBlobSha } = await import("./lib/plan-sha.ts");
  const abs = resolve(cwd, planPath);
  const text = readFileSync(abs, "utf8");
  const rows = pinnedAdrs(text);
  const files = rows.map((r) => {
    const f = decisionFile(cwd, decisions, r.file);
    if (f === null) {
      throw new Error(`ADR file outside ${decisions}: ${r.file}`);
    }
    return f;
  });
  const { keepBlob } = await import("./lib/git.ts");
  const blobs = new Map(
    rows.map((r, i) => {
      const bytes = readFileSync(files[i] as string);
      // The pinned text stays readable by its blob sha, uncommitted, so a handoff that stopped
      // after writing the acceptance can compare against it (#212). Best effort: outside a git
      // checkout the pin still holds, and the handoff then counts the real blob.
      try {
        keepBlob(cwd, bytes);
      } catch {
        // not a git checkout
      }
      return [r.adr, gitBlobSha(bytes)] as const;
    }),
  );
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

/** An ADR's status word: `proposed`, `accepted`, `superseded`, … */
export function adrStatus(text: string): string {
  return field(text, "Status").split(/\s+/)[0] ?? "";
}

/**
 * The ADR moved to accepted (ADR 0035, ADR 0055): *Decided by* names who accepted it and how,
 * and keeps who drafted it. Shared by `adrs.ts accept` and `plan.ts handoff`.
 */
export function acceptedText(text: string, acceptedBy: string): string {
  const decider = field(text, "Decided by").replace(/;\s*a human accepts\s*$/, "");
  return text
    .replace(/^- \*\*Status:\*\*\s*proposed\b.*$/m, "- **Status:** accepted")
    .replace(/^- \*\*Decided by:\*\*.*$/m, `- **Decided by:** ${acceptedBy}; drafted by ${decider}`);
}

/** How `plan.ts handoff` names an acceptance by a Rubato approval: approver, date, link, self. */
export function rubatoAcceptance(by: string, date: string, url: string, self: boolean): string {
  return `${by}, accepted ${date}, approved in Rubato ${url}${self ? " (self)" : ""}`;
}

/** Whether the ADR is accepted by the Rubato approval at `url`: a handoff already wrote it. */
export function acceptedInRubato(text: string, url: string): boolean {
  return adrStatus(text) === "accepted" && field(text, "Decided by").includes(`approved in Rubato ${url}`);
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
  // Only inside a map repo's checkout: the one of the listed maps whose repo this is.
  const mapClone = ws.maps.find((m) => m.repo.toLowerCase() === (originRepo(cwd) ?? "").toLowerCase());
  if (!mapClone) {
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
  const decisions = mapClone.decisions ?? "docs/decisions";
  for (const r of rows) {
    const abs = decisionFile(cwd, decisions, r.file);
    if (abs === null) {
      refused.push({ adr: r.adr, reason: "outside_decisions" });
      continue;
    }
    if (!existsSync(abs)) {
      refused.push({ adr: r.adr, reason: "missing" });
      continue;
    }
    const text = readFileSync(abs, "utf8");
    if (gitBlobSha(text) !== r.blob) {
      refused.push({ adr: r.adr, reason: "adr_changed" });
      continue;
    }
    if (adrStatus(text) !== "proposed") {
      refused.push({ adr: r.adr, reason: "not_proposed" });
      continue;
    }
    writes.push([abs, acceptedText(text, `${c.user.login}, accepted ${today} by approval comment ${c.html_url}${self ? " (self)" : ""}`)]);
    accepted.push(r.adr);
  }
  const pinned = new Set(rows.map((r) => r.file));
  const dir = join(cwd, decisions);
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
      // Retired at the cutover (ADR 0054, ADR 0055): approving a plan in Rubato accepts its pinned
      // ADRs, and plan.ts handoff records that in the handoff commit.
      console.log(JSON.stringify({ event: "rubato.adrs.accept", outcome: "refused", reason: "approval_in_rubato" }));
      return 11;
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
  const argv = process.argv.slice(2);
  const { runCommand, subcommandOf } = await import("./otel-emit.ts");
  process.exitCode = await runCommand("rubato", subcommandOf("adrs", argv), () => main(argv));
}
