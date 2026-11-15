/**
 * The files that travel with a plan, and the hash of all of them (plan 122, design §D3). The
 * TypeScript twin of `rubato_protocol::plan_hash::{bundle_paths, bundle_hash}`; both read
 * `schema/fixtures/plans/bundle/`.
 */
import { createHash } from "node:crypto";
import { gitBlobSha, planSha } from "./plan-sha.ts";

/** `a/./b/../c` → `a/c`; null when the path leaves the repository or is empty. */
function normalisePath(path: string): string | null {
  const parts: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "" || seg === ".") {
      continue;
    }
    if (seg === "..") {
      if (parts.pop() === undefined) {
        return null;
      }
      continue;
    }
    parts.push(seg);
  }
  return parts.length ? parts.join("/") : null;
}

/** Relative link targets `[text](target)` on one line: no scheme, not absolute, no anchor. */
function linkTargets(line: string): string[] {
  const out: string[] = [];
  let rest = line;
  for (;;) {
    const i = rest.indexOf("](");
    if (i < 0) {
      break;
    }
    const after = rest.slice(i + 2);
    const end = after.indexOf(")");
    if (end < 0) {
      break;
    }
    const raw = after.slice(0, end).trim();
    const first = raw.includes(" ") ? raw.slice(0, raw.indexOf(" ")) : raw;
    const target = (first.replace(/^<+|>+$/g, "").split(/[#?]/)[0] ?? "").trim();
    if (target !== "" && !target.startsWith("/") && !target.includes("://")) {
      out.push(target);
    }
    rest = after.slice(end);
  }
  return out;
}

function splitRow(line: string): string[] {
  const cells: string[] = [];
  let cur = "";
  let inCode = false;
  for (const ch of line.trim()) {
    if (ch === "`") {
      inCode = !inCode;
    }
    if (ch === "|" && !inCode) {
      cells.push(cur);
      cur = "";
    } else {
      cur += ch;
    }
  }
  cells.push(cur);
  if (cells.length && cells[0]?.trim() === "") {
    cells.shift();
  }
  if (cells.length && cells[cells.length - 1]?.trim() === "") {
    cells.pop();
  }
  return cells.map((c) => c.trim());
}

/** The File column of the `## ADRs proposed` table, backticks removed. */
function adrFiles(text: string): string[] {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const start = lines.findIndex((l) => {
    const h = l.replace(/^#+/, "");
    return h.length < l.length && h.startsWith(" ") && h.trim().toLowerCase().startsWith("adrs proposed");
  });
  if (start < 0) {
    return [];
  }
  let i = start + 1;
  while (i < lines.length && !(lines[i] as string).startsWith("#") && !(lines[i] as string).trimStart().startsWith("|")) {
    i++;
  }
  const rows: string[][] = [];
  while (i < lines.length && (lines[i] as string).trimStart().startsWith("|")) {
    rows.push(splitRow(lines[i] as string));
    i++;
  }
  const header = rows[0];
  if (!header) {
    return [];
  }
  const col = header.findIndex((h) => h.toLowerCase().startsWith("file"));
  if (col < 0) {
    return [];
  }
  return rows
    .slice(2)
    .map((r) => (r[col] ?? "").replace(/`/g, "").trim())
    .filter((c) => c !== "");
}

/**
 * The plan's bundle as repository paths: the plan first, then, sorted and without duplicates,
 * the relative links (outside code fences) into `<plan dir>/<plan stem>-…/` and the File column of
 * `## ADRs proposed`. Paths that leave the repository are dropped.
 */
export function bundlePaths(planText: string, planPathIn: string): string[] {
  const planPath = normalisePath(planPathIn) ?? planPathIn;
  const slash = planPath.lastIndexOf("/");
  const dir = slash >= 0 ? planPath.slice(0, slash) : "";
  const stem = (slash >= 0 ? planPath.slice(slash + 1) : planPath).replace(/\.md$/, "");
  const prefix = dir ? `${dir}/${stem}-` : `${stem}-`;
  const found = new Set<string>();
  let inFence = false;
  for (const raw of planText.split("\n")) {
    const line = raw.replace(/\r$/, "");
    if (line.trimStart().startsWith("```")) {
      inFence = !inFence;
      continue;
    }
    if (inFence) {
      continue;
    }
    for (const target of linkTargets(line)) {
      const p = normalisePath(dir ? `${dir}/${target}` : target);
      if (p && p.startsWith(prefix) && p.slice(prefix.length).includes("/")) {
        found.add(p);
      }
    }
  }
  for (const f of adrFiles(planText)) {
    const p = normalisePath(f);
    if (p) {
      found.add(p);
    }
  }
  found.delete(planPath);
  // Sort by UTF-16 code unit, as Rust's BTreeSet sorts by byte for ASCII paths.
  return [planPath, ...[...found].sort()];
}

/** sha1 over the sorted lines `<path>\0<blob sha>\n`; the plan's blob sha is its approval hash. */
export function bundleHash(planPath: string, files: Array<[string, string]>): string {
  const lines = files
    .map(([path, text]) => [path, path === planPath ? planSha(text) : gitBlobSha(text)] as [string, string])
    .sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : a[1] < b[1] ? -1 : a[1] > b[1] ? 1 : 0));
  const h = createHash("sha1");
  for (const [path, blob] of lines) {
    h.update(`${path}\0${blob}\n`);
  }
  return h.digest("hex");
}
