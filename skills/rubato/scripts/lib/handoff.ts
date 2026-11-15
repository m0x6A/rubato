/**
 * `plan.ts handoff` (ADR 0055, plan 122 design §D7): after a team member approved the draft in
 * Rubato, write the approval record into the plan, accept the plan's pinned ADRs in the map repo,
 * make one commit of the plan and its bundle, push it, and report one `rubato.plan.handoff` line
 * (`rubato_protocol::HandoffLine`) on stdout and to the companion's loopback `/handoff`.
 *
 * Deterministic, and safe to run again: a commit carrying `Rubato-Approval: <id>` is the handoff
 * already made, so a rerun only pushes it. Every refusal comes before the first write.
 */
import { existsSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { join, resolve, sep } from "node:path";
import { acceptedInRubato, acceptedText, adrStatus, pinnedAdrs, rubatoAcceptance } from "../adrs.ts";
import { planIssue } from "./approval.ts";
import { currentBranch, git, issueFromBranch, originRepo } from "./git.ts";
import type { Workspace } from "./local.ts";
import { APPROVAL_KEYS, gitBlobSha, planSha } from "./plan-sha.ts";
import { bundleModule } from "./vendor.ts";

/** One exit code per outcome, as `plan.ts wait` has (`plan_changed` is 7 in both). */
export const HANDOFF_EXIT = { committed: 0, error: 2, plan_changed: 7, bundle_changed: 8, adr_changed: 9, push_failed: 10 } as const;

export type HandoffOutcome = keyof typeof HANDOFF_EXIT;

/** The `rubato.plan.handoff` line: `rubato_protocol::HandoffLine`. */
export interface HandoffLine {
  event: "rubato.plan.handoff";
  outcome: HandoffOutcome;
  approval: string;
  commit?: string;
  adrs_accepted: string[];
  adrs_left: string[];
  error?: string;
}

/** The approval record as the companion types it (`rubato_protocol::handoff_prompt`). */
export interface HandoffArgs {
  approval: string;
  sha: string;
  bundle: string;
  by: string;
  at: string;
  self: boolean;
  url: string;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const RULES: Record<Exclude<keyof HandoffArgs, "self">, [RegExp, string]> = {
  approval: [UUID, "a UUID"],
  sha: [/^[0-9a-f]{7,40}$/i, "7 to 40 hex characters"],
  bundle: [/^[0-9a-f]+$/i, "hex"],
  by: [/^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/, "a GitHub login"],
  at: [/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/, "an RFC 3339 time"],
  url: [/^https?:\/\/\S+$/, "an http(s) URL"],
};

/** `--approval <uuid> --sha <sha> --bundle <hash> --by <login> --at <rfc3339> [--self] --url <url>`. */
export function parseHandoffArgs(argv: string[]): { ok: true; args: HandoffArgs } | { ok: false; error: string; approval?: string } {
  const at = (name: string) => {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const bad: string[] = [];
  const got: Partial<Record<keyof typeof RULES, string>> = {};
  for (const [name, [re, what]] of Object.entries(RULES) as [keyof typeof RULES, [RegExp, string]][]) {
    const v = at(name);
    if (v === undefined || v.startsWith("--") || !re.test(v)) {
      bad.push(`--${name} must be ${what}`);
    } else {
      got[name] = v;
    }
  }
  if (bad.length) {
    return { ok: false, error: bad.join("; "), ...(got.approval ? { approval: got.approval.toLowerCase() } : {}) };
  }
  const g = got as Record<keyof typeof RULES, string>;
  return { ok: true, args: { approval: g.approval.toLowerCase(), sha: g.sha.toLowerCase(), bundle: g.bundle.toLowerCase(), by: g.by, at: g.at, self: argv.includes("--self"), url: g.url } };
}

/**
 * The plan with the approval record in its front matter, quoted, after the other keys. Keys
 * already there are replaced, so equal keys give the same text and a new approval rewrites them.
 * The approval hash leaves these lines out, so the text still hashes to the sha it records.
 */
export function withApprovalRecord(text: string, record: Record<(typeof APPROVAL_KEYS)[number], string>): string {
  const lines = text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  if ((lines[0] ?? "").trim() !== "---") {
    throw new Error("the plan has no front matter to hold the approval record");
  }
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  if (end < 0) {
    throw new Error("the plan's front matter is not closed with ---");
  }
  const nl = (lines[0] as string).endsWith("\r\n") ? "\r\n" : "\n";
  const kept = lines.slice(1, end).filter((l) => !APPROVAL_KEYS.some((k) => l.startsWith(`${k}:`)));
  const record_ = APPROVAL_KEYS.map((k) => `${k}: ${JSON.stringify(record[k])}${nl}`);
  return [lines[0], ...kept, ...record_, ...lines.slice(end)].join("");
}

/** The bundle's files that exist as regular files inside the worktree, as the companion reads them. */
function presentFiles(top: string, paths: string[]): [string, string][] {
  const realTop = realpathSync(top);
  return paths.flatMap((p) => {
    const abs = join(top, p);
    try {
      const real = realpathSync(abs);
      if (!(real === realTop || real.startsWith(realTop + sep)) || !statSync(real).isFile()) {
        return [];
      }
      return [[p, readFileSync(real, "utf8")] as [string, string]];
    } catch {
      return [];
    }
  });
}

/** The commit on this branch that already carries `Rubato-Approval: <id>`, if any. */
function handoffCommit(top: string, approval: string): string | null {
  const out = git(top, "log", "-F", `--grep=Rubato-Approval: ${approval}`, "--format=%H", "-n", "1", "HEAD");
  return out || null;
}

function errorText(e: unknown): string {
  const any = e as { stderr?: unknown; message?: unknown };
  const stderr = typeof any.stderr === "string" ? any.stderr.trim() : any.stderr instanceof Buffer ? any.stderr.toString().trim() : "";
  return (stderr || (typeof any.message === "string" ? any.message : String(e))).split("\n").slice(-3).join(" ").slice(0, 500);
}

export interface HandoffOptions {
  /** Where the line goes; one call, one line. Default stdout. */
  out?: (line: string) => void;
  /** Diagnostics, as JSON lines. Default stderr. */
  err?: (line: string) => void;
  /** `RUBATO_HOOK_URL`, `RUBATO_HOOK_TOKEN` and `RUBATO_LAUNCH_ID`. Default `process.env`. */
  env?: Record<string, string | undefined>;
  /**
   * The plan path for the branch, as `plan.ts path` finds it. `plan.ts` passes its own: a dynamic
   * import of `plan.ts` from inside its top-level `await main()` would wait on itself forever.
   */
  planPath?: (ws: Workspace, cwd: string) => Promise<string>;
}

/** Post the line to the companion's loopback `/handoff`, as the hooks post theirs. Never throws. */
export async function report(line: HandoffLine, env: Record<string, string | undefined>): Promise<Record<string, unknown>> {
  const base = env["RUBATO_HOOK_URL"];
  const token = env["RUBATO_HOOK_TOKEN"];
  if (!base || !token) {
    return { sent: false, reason: "no_endpoint" };
  }
  const headers: Record<string, string> = { Authorization: `Bearer ${token}`, "Content-Type": "application/json" };
  if (env["RUBATO_LAUNCH_ID"]) {
    headers["X-Rubato-Launch"] = env["RUBATO_LAUNCH_ID"];
  }
  try {
    const res = await fetch(`${base.replace(/\/+$/, "")}/handoff`, { method: "POST", headers, body: JSON.stringify(line), signal: AbortSignal.timeout(3000) });
    return { sent: res.ok, status: res.status };
  } catch (e) {
    return { sent: false, error: errorText(e) };
  }
}

/** Run the handoff, print its line, report it, and give the exit code. */
export async function handoff(ws: Workspace, cwd: string, a: HandoffArgs, o: HandoffOptions = {}): Promise<{ code: number; line: HandoffLine }> {
  const out = o.out ?? ((l: string) => console.log(l));
  const err = o.err ?? ((l: string) => console.error(l));
  const line = await run(ws, cwd, a, o).catch((e: unknown): HandoffLine => ({ event: "rubato.plan.handoff", outcome: "error", approval: a.approval, adrs_accepted: [], adrs_left: [], error: errorText(e) }));
  out(JSON.stringify(line));
  const sent = await report(line, o.env ?? process.env);
  err(JSON.stringify({ event: "rubato.plan.handoff.report", outcome: line.outcome, ...sent }));
  return { code: HANDOFF_EXIT[line.outcome], line };
}

async function run(ws: Workspace, cwd: string, a: HandoffArgs, o: HandoffOptions): Promise<HandoffLine> {
  const done = (outcome: HandoffOutcome, fields: Partial<HandoffLine> = {}): HandoffLine => ({ event: "rubato.plan.handoff", outcome, approval: a.approval, adrs_accepted: [], adrs_left: [], ...fields });
  const top = git(cwd, "rev-parse", "--show-toplevel");
  const find = o.planPath ?? (await import("../plan.ts")).planPathFromBranch;
  const planPath = await find(ws, top);
  const planAbs = resolve(top, planPath);
  if (!existsSync(planAbs)) {
    throw new Error(`no plan at ${planPath}`);
  }
  const text = readFileSync(planAbs, "utf8");
  const now = planSha(text);
  if (!now.startsWith(a.sha)) {
    return done("plan_changed", { error: `${planPath} hashes to ${now}, the approval names ${a.sha}` });
  }

  const pinned = pinnedAdrs(text);
  const mapRepo = ws.maps.some((m) => m.repo.toLowerCase() === (originRepo(top) ?? "").toLowerCase());
  const branch = currentBranch(top);
  if (branch === "HEAD") {
    throw new Error("HEAD is detached; check out the plan's branch");
  }
  const push = (commit: string, accepted: string[], left: string[]): HandoffLine => {
    try {
      git(top, "push", "-q", "origin", `HEAD:refs/heads/${branch}`);
    } catch (e) {
      return done("push_failed", { commit, adrs_accepted: accepted, adrs_left: left, error: errorText(e) });
    }
    return done("committed", { commit, adrs_accepted: accepted, adrs_left: left });
  };

  // A rerun: the handoff commit exists (its push may have failed). Push it again; write nothing.
  const made = handoffCommit(top, a.approval);
  if (made) {
    const accepted = mapRepo ? pinned.filter((r) => existsSync(join(top, r.file)) && acceptedInRubato(readFileSync(join(top, r.file), "utf8"), a.url)).map((r) => r.adr) : [];
    return push(made, accepted, pinned.map((r) => r.adr).filter((n) => !accepted.includes(n)));
  }

  // The bundle as approved. An ADR this approval already accepted (a run that stopped before its
  // commit) counts at the blob the plan pinned, which is what the approval saw.
  const bundle = await bundleModule();
  const paths = bundle.bundlePaths(text, planPath);
  const files = presentFiles(top, paths);
  const pinnedBlob = new Map(pinned.map((r) => [r.file, r.blob]));
  const blobs = files.map(([p, t]): [string, string] => {
    if (p === planPath) {
      return [p, planSha(t)];
    }
    const pin = pinnedBlob.get(p);
    return [p, mapRepo && pin && acceptedInRubato(t, a.url) ? pin : gitBlobSha(t)];
  });
  const bundleNow = bundle.bundleHashOfBlobs(blobs);
  if (bundleNow !== a.bundle) {
    return done("bundle_changed", { error: `the bundle hashes to ${bundleNow}, the approval names ${a.bundle}` });
  }

  // The ADRs: every check before any write.
  const accepted: string[] = [];
  const left: string[] = [];
  const writes: [string, string][] = [];
  const changed: string[] = [];
  const date = a.at.slice(0, 10);
  for (const r of pinned) {
    const abs = join(top, r.file);
    if (!mapRepo || !existsSync(abs)) {
      left.push(r.adr);
      continue;
    }
    const t = readFileSync(abs, "utf8");
    if (acceptedInRubato(t, a.url)) {
      accepted.push(r.adr);
    } else if (gitBlobSha(t) !== r.blob) {
      changed.push(r.adr);
    } else if (adrStatus(t) !== "proposed") {
      left.push(r.adr);
    } else {
      writes.push([abs, acceptedText(t, rubatoAcceptance(a.by, date, a.url, a.self))]);
      accepted.push(r.adr);
    }
  }
  if (changed.length) {
    return done("adr_changed", { error: `ADR ${changed.join(", ")} changed after it was pinned: pin again and ask for a new approval` });
  }

  // Write the record, then the ADRs, then commit at once: the plan and every bundle path present.
  const next = withApprovalRecord(text, { approved_by: a.by, approved_at: a.at, approved_sha: now });
  if (next !== text) {
    writeFileSync(planAbs, next);
  }
  for (const [abs, t] of writes) {
    writeFileSync(abs, t);
  }
  const commitPaths = files.map(([p]) => p);
  const n = planIssue(next)?.number ?? issueFromBranch(branch);
  git(top, "add", "--", ...commitPaths);
  git(top, "commit", "-q", "--only", "--allow-empty", "-m", `Plan #${n} approved`, "-m", `Approved-by: ${a.by}\nRubato-Approval: ${a.approval}`, "--", ...commitPaths);
  return push(git(top, "rev-parse", "HEAD"), accepted, left);
}

/**
 * `plan.ts handoff …` from the command line. Arguments or a workspace that cannot be read still
 * end in a line (`error`) whenever the approval id is readable, so the companion hears of it.
 */
export async function handoffMain(argv: string[], cwd: string, loadWs: () => Workspace, o: HandoffOptions = {}): Promise<number> {
  const out = o.out ?? ((l: string) => console.log(l));
  const err = o.err ?? ((l: string) => console.error(l));
  const fail = async (approval: string | undefined, error: string): Promise<number> => {
    err(error);
    if (approval) {
      const line: HandoffLine = { event: "rubato.plan.handoff", outcome: "error", approval, adrs_accepted: [], adrs_left: [], error };
      out(JSON.stringify(line));
      err(JSON.stringify({ event: "rubato.plan.handoff.report", outcome: "error", ...(await report(line, o.env ?? process.env)) }));
    }
    return HANDOFF_EXIT.error;
  };
  const p = parseHandoffArgs(argv);
  if (!p.ok) {
    return fail(p.approval, `usage: plan.ts handoff --approval <uuid> --sha <plan sha> --bundle <bundle hash> --by <login> --at <rfc3339> [--self] --url <rubato url>: ${p.error}`);
  }
  let ws: Workspace;
  try {
    ws = loadWs();
  } catch (e) {
    return fail(p.args.approval, errorText(e));
  }
  return (await handoff(ws, cwd, p.args, o)).code;
}
