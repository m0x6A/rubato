/**
 * The I/O behind waiting for an approval and accepting a plan's ADRs (ADR 0035): the plan as
 * pushed (git) and the issue's comments (the developer's own `gh`, seam `skill-github`). Rules
 * live in `comments.ts`; this file only fetches.
 */
import { execFileSync } from "node:child_process";
import { currentBranch, git } from "./git.ts";
import type { GhComment } from "./comments.ts";

/** One `rubato.plan.wait` or `rubato.adrs.accept` line, printed as JSON (names in otel.rs). */
export function line(event: "rubato.plan.wait" | "rubato.adrs.accept", fields: Record<string, unknown>): string {
  return JSON.stringify({ event, ...fields });
}

/** The issue a plan's front matter names: `issue: owner/name#n`. */
export function planIssue(text: string): { repo: string; number: number } | null {
  const front = /^---\n([\s\S]*?)\n---/.exec(text)?.[1] ?? "";
  const m = /^issue:\s*([^\s/#]+\/[^\s/#]+)#(\d+)\s*$/m.exec(front);
  return m ? { repo: m[1] as string, number: Number(m[2]) } : null;
}

export type Pushed = { ok: true; branch: string; text: string } | { ok: false; reason: "not_pushed" | "local_edit"; branch: string };

/**
 * The plan at `path` as the branch on `origin` has it, which is what the server reads. Fetches
 * the branch first. `not_pushed` when origin has no such branch or file; `local_edit` when the
 * working tree's file differs from it.
 */
export function pushedPlan(cwd: string, path: string, local: string): Pushed {
  const branch = currentBranch(cwd);
  let text: string;
  try {
    git(cwd, "fetch", "-q", "origin", branch);
    text = execFileSync("git", ["-C", cwd, "show", `origin/${branch}:${path}`], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    return { ok: false, reason: "not_pushed", branch };
  }
  if (text !== local) {
    return { ok: false, reason: "local_edit", branch };
  }
  return { ok: true, branch, text };
}

function gh(args: string[]): string {
  return execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

/** Every comment on the issue, all pages (`--paginate`), one JSON object per line. */
export function issueComments(repo: string, number: number): GhComment[] {
  return gh(["api", "--paginate", `repos/${repo}/issues/${number}/comments`, "--jq", ".[]"])
    .split("\n")
    .filter((l) => l.trim() !== "")
    .map((l) => JSON.parse(l) as GhComment);
}

/** One comment by its id, read again from GitHub. */
export function issueComment(repo: string, id: number): GhComment & { issue_url: string } {
  return JSON.parse(gh(["api", `repos/${repo}/issues/comments/${id}`])) as GhComment & { issue_url: string };
}

/** The login the developer's `gh` acts as, to mark a self-approval. */
export function ghLogin(): string {
  return gh(["api", "user", "--jq", ".login"]).trim();
}

/** The comment id in `…#issuecomment-<id>`, or a bare id. */
export function commentId(urlOrId: string): number | null {
  const m = /(?:issuecomment-)?(\d+)$/.exec(urlOrId.trim());
  return m ? Number(m[1]) : null;
}

/** Why a `gh` call failed, in one line: exit code and first stderr line. */
export function ghFailure(e: unknown): { code: number | null; message: string } {
  const err = e as { status?: number; stderr?: string | Buffer; message?: string };
  const stderr = err.stderr ? String(err.stderr).split("\n")[0] : undefined;
  return { code: err.status ?? null, message: stderr || err.message || String(e) };
}
