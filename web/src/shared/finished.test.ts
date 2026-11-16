import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { issue, overview } from "../../test/fixtures.ts";
import { closedUnmerged, finishedIssues, isFinished, openIssues } from "./finished.ts";

const now = new Date().toISOString();
const at = (n: number, state: "designed" | "done") => ({ issue: { repo: "m0x6A/rubato", number: n }, state, last_seen: now });

describe("finished is the server's word (#265)", () => {
  it("reads `finished` and derives nothing", () => {
    const open = issue(1, "web", { progress: at(1, "designed") });
    const closedWorking = issue(2, "web", { progress: at(2, "designed"), closed: true });
    const closed = issue(3, "web", { progress: at(3, "designed"), closed: true, finished: true });
    // What an older server would send for a merged issue: no `finished`, so it reads as open.
    const mergedOldServer = issue(4, "web", { progress: at(4, "done"), finished: false });
    const merged = issue(5, "web", { progress: at(5, "done") });
    const ov = overview({ issues: [open, closedWorking, closed, mergedOldServer, merged] });
    expect([open, closedWorking, closed, mergedOldServer, merged].map(isFinished)).toEqual([false, false, true, false, true]);
    expect(openIssues(ov).map((i) => i.issue.number)).toEqual([1, 2, 4]);
    expect(finishedIssues(ov).map((i) => i.issue.number)).toEqual([5, 3]);
    expect([closedWorking, closed, merged].map(closedUnmerged)).toEqual([true, true, false]);
  });

  it("lists the finished newest first, by merge time", () => {
    const pr = (merged_at: string) => ({ repo: "m0x6A/rubato", number: 9, branch: "b", open: false, draft: false, checks: "green" as const, merged_at, updated_at: merged_at, url: "u" });
    const ov = overview({
      issues: [issue(1, "web", { progress: at(1, "done"), pull_request: pr("2026-11-01T00:00:00Z") }), issue(2, "web", { progress: at(2, "done"), pull_request: pr("2026-11-10T00:00:00Z") })],
    });
    expect(finishedIssues(ov).map((i) => i.issue.number)).toEqual([2, 1]);
  });
});

/** Lines that decide by themselves whether an issue is done: a comparison with "done", or a
 * test of `merged_at`. Every list must ask `isFinished` instead. The Open / Done switch of a
 * list compares its own `view`, which is not a rule about an issue. */
function ownRules(text: string): string[] {
  return text
    .split("\n")
    .map((l) => l.replace(/\bview\s*[!=]==\s*"done"/g, ""))
    .filter((l) => /[!=]==\s*"done"|"done"\s*[!=]==|merged_at/.test(l) && !/^\s*(\/\/|\*|\/\*)/.test(l));
}

/** Files that may name "done" or `merged_at`, and why. Anything else is a second rule. */
const ALLOWED: Record<string, string> = {
  "shared/finished.ts": "the helper: when it finished, and closed without a merge",
  "features/issues/IssuePanel.tsx": "labels: the pull request line and the finished note",
  "features/pulls/PullView.tsx": "a pull request's own merge time",
  "protocol.ts": "generated types",
};

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      return sources(p);
    }
    return /\.tsx?$/.test(e.name) && !/\.test\.tsx?$/.test(e.name) ? [p] : [];
  });
}

describe("no reader keeps a done rule of its own (#265)", () => {
  const root = join(import.meta.dirname, "..");

  it("finds none outside the allowlist", () => {
    const files = sources(root);
    const offenders = files.map((f) => ({ file: relative(root, f), lines: ownRules(readFileSync(f, "utf8")) })).filter((f) => f.lines.length > 0 && !(f.file in ALLOWED));
    console.log(`checked ${files.length} source files for a done rule of their own; allowed: ${Object.keys(ALLOWED).join(", ")}`);
    expect(files.length).toBeGreaterThan(50);
    expect(offenders).toEqual([]);
  });

  it("must fail: the filters this replaced are found", () => {
    expect(ownRules('const open = ov.issues.filter((i) => i.progress.state !== "done");')).toHaveLength(1);
    expect(ownRules("  .filter((i) => !i.pull_request?.merged_at)")).toHaveLength(1);
    expect(ownRules('  return state !== "done";')).toHaveLength(1);
    expect(ownRules('  // a comment about "done" === nothing\n  { state: "done", label: "Done" },')).toHaveLength(0);
    expect(ownRules('<button aria-pressed={view === "done"}>')).toHaveLength(0);
    expect(ownRules('<li hidden={view === "done" && i.progress.state === "done"}>')).toHaveLength(1);
  });
});
