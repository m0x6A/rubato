/**
 * A line diff between two revisions of a plan's bundle, for "what changed since you last looked"
 * (ADR 0053). The longest common subsequence of lines, after the common head and tail are set
 * aside, so an edit in a long plan compares only the lines around it.
 */
import type { DraftFile } from "../../protocol.ts";

export type Op = "same" | "add" | "del";

export interface DiffLine {
  op: Op;
  text: string;
}

/** The LCS table of the changed middle may hold at most this many cells (4 bytes each). */
export const MAX_CELLS = 2_000_000;

/** A text's lines, on LF or CRLF. A final newline does not make an extra empty line. */
export function splitLines(text: string): string[] {
  if (text === "") {
    return [];
  }
  const lines = text.split(/\r?\n/);
  if (lines[lines.length - 1] === "") {
    lines.pop();
  }
  return lines;
}

const line = (op: Op) => (text: string): DiffLine => ({ op, text });

/**
 * The lines of `b` against `a`. A removed line comes before the line added in its place. Past
 * `maxCells` the changed middle is shown removed, then added, and `exact` is false.
 */
export function diffLines(a: string[], b: string[], maxCells = MAX_CELLS): { lines: DiffLine[]; exact: boolean } {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) {
    start += 1;
  }
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA -= 1;
    endB -= 1;
  }
  const head = a.slice(0, start).map(line("same"));
  const tail = a.slice(endA).map(line("same"));
  const x = a.slice(start, endA);
  const y = b.slice(start, endB);
  const n = x.length;
  const m = y.length;
  if (n === 0 || m === 0) {
    return { lines: [...head, ...x.map(line("del")), ...y.map(line("add")), ...tail], exact: true };
  }
  if ((n + 1) * (m + 1) > maxCells) {
    return { lines: [...head, ...x.map(line("del")), ...y.map(line("add")), ...tail], exact: false };
  }
  // lcs[i * w + j]: the length of the longest common subsequence of x[i..] and y[j..].
  const w = m + 1;
  const lcs = new Uint32Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      lcs[i * w + j] = x[i] === y[j] ? (lcs[(i + 1) * w + j + 1] as number) + 1 : Math.max(lcs[(i + 1) * w + j] as number, lcs[i * w + j + 1] as number);
    }
  }
  const mid: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      mid.push({ op: "same", text: x[i] as string });
      i += 1;
      j += 1;
    } else if ((lcs[(i + 1) * w + j] as number) >= (lcs[i * w + j + 1] as number)) {
      mid.push({ op: "del", text: x[i] as string });
      i += 1;
    } else {
      mid.push({ op: "add", text: y[j] as string });
      j += 1;
    }
  }
  for (; i < n; i += 1) {
    mid.push({ op: "del", text: x[i] as string });
  }
  for (; j < m; j += 1) {
    mid.push({ op: "add", text: y[j] as string });
  }
  return { lines: [...head, ...mid, ...tail], exact: true };
}

/** A run of unchanged lines folded away, with how many it hides. */
export interface Gap {
  gap: number;
}

/** The changes with `context` unchanged lines around each; a longer unchanged run is one gap. */
export function hunks(lines: DiffLine[], context = 3): (DiffLine | Gap)[] {
  const near = new Array<boolean>(lines.length).fill(false);
  lines.forEach((l, k) => {
    if (l.op !== "same") {
      for (let d = Math.max(0, k - context); d <= Math.min(lines.length - 1, k + context); d += 1) {
        near[d] = true;
      }
    }
  });
  const out: (DiffLine | Gap)[] = [];
  let k = 0;
  while (k < lines.length) {
    if (near[k]) {
      out.push(lines[k] as DiffLine);
      k += 1;
      continue;
    }
    let end = k;
    while (end < lines.length && !near[end]) {
      end += 1;
    }
    // Folding a single line hides nothing worth the marker that replaces it.
    if (end - k >= 2) {
      out.push({ gap: end - k });
    } else {
      out.push(...lines.slice(k, end));
    }
    k = end;
  }
  return out;
}

/** The plan (a bundle's first file, as the companion sends it) first, then the rest by path. */
export function bundleOrder<T extends { path: string }>(files: T[]): T[] {
  const [plan, ...rest] = files;
  return plan ? [plan, ...[...rest].sort((p, q) => (p.path < q.path ? -1 : p.path > q.path ? 1 : 0))] : [];
}

export interface FileChange {
  path: string;
  status: "added" | "removed" | "changed";
  lines: DiffLine[];
  added: number;
  removed: number;
  exact: boolean;
}

/** What changed from `before` to `after`, file by file, plan first; unchanged files are left out. */
export function diffBundles(before: DraftFile[], after: DraftFile[]): FileChange[] {
  const old = new Map(before.map((f) => [f.path, f.text]));
  const now = new Map(after.map((f) => [f.path, f.text]));
  const plan = after[0]?.path ?? before[0]?.path;
  const others = [...new Set([...old.keys(), ...now.keys()])].filter((p) => p !== plan);
  const order = bundleOrder([...(plan === undefined ? [] : [{ path: plan }]), ...others.map((path) => ({ path }))]);
  const changes: FileChange[] = [];
  for (const { path } of order) {
    const a = old.get(path);
    const b = now.get(path);
    if (a === b) {
      continue;
    }
    const { lines, exact } = diffLines(splitLines(a ?? ""), splitLines(b ?? ""));
    changes.push({
      path,
      status: a === undefined ? "added" : b === undefined ? "removed" : "changed",
      lines,
      added: lines.filter((l) => l.op === "add").length,
      removed: lines.filter((l) => l.op === "del").length,
      exact,
    });
  }
  return changes;
}
