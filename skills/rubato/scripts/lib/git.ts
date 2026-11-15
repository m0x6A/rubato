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

/** The text of a blob in this checkout's object store, or null when git does not have it. */
export function blobText(cwd: string, sha: string): string | null {
  if (!/^[0-9a-f]{40}$/.test(sha)) {
    return null;
  }
  try {
    return execFileSync("git", ["-C", cwd, "cat-file", "blob", sha], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return null;
  }
}

/**
 * Write a text into the object store as a loose object, uncommitted and exactly as it is (no
 * filters), so its blob sha can be read back later (#212). Gives the sha git stored it under. Unreachable objects stay until
 * `git gc` prunes them, two weeks by default.
 */
export function keepBlob(cwd: string, text: string | Buffer): string {
  return execFileSync("git", ["-C", cwd, "hash-object", "-w", "--no-filters", "--stdin"], { input: text, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] }).trim();
}
