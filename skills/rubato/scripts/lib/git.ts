import { execFileSync } from "node:child_process";

export function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

export function headSha(cwd: string): string {
  return git(cwd, "rev-parse", "--short", "HEAD");
}

export function currentBranch(cwd: string): string {
  return git(cwd, "rev-parse", "--abbrev-ref", "HEAD");
}

/** `owner/name` from the origin URL, or null. */
export function originRepo(cwd: string): string | null {
  let url: string;
  try {
    url = git(cwd, "remote", "get-url", "origin");
  } catch {
    return null;
  }
  const m = /[:/]([^/:]+\/[^/]+?)(\.git)?$/.exec(url);
  return m ? (m[1] as string) : null;
}

/** Issue number from `issue/<n>-<slug>` (ADR 0006), or null. */
export function issueFromBranch(branch: string): number | null {
  const m = /^issue\/(\d+)(-|$)/.exec(branch);
  return m ? Number(m[1]) : null;
}
