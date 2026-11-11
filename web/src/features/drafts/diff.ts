/** A line diff of two texts (longest common subsequence), for showing what a proposal changes. */
export interface DiffLine {
  sign: " " | "+" | "-";
  text: string;
}

export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  // lcs[i][j]: length of the longest common subsequence of a[i..] and b[j..].
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i]![j] = a[i] === b[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      out.push({ sign: " ", text: a[i]! });
      i++;
      j++;
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) {
      out.push({ sign: "-", text: a[i++]! });
    } else {
      out.push({ sign: "+", text: b[j++]! });
    }
  }
  while (i < a.length) {
    out.push({ sign: "-", text: a[i++]! });
  }
  while (j < b.length) {
    out.push({ sign: "+", text: b[j++]! });
  }
  return out;
}

/** Only the changed lines with `context` unchanged lines around each run; gaps as a `…` line. */
export function changedWithContext(diff: DiffLine[], context = 2): DiffLine[] {
  const keep = new Set<number>();
  diff.forEach((l, at) => {
    if (l.sign !== " ") {
      for (let k = Math.max(0, at - context); k <= Math.min(diff.length - 1, at + context); k++) {
        keep.add(k);
      }
    }
  });
  const out: DiffLine[] = [];
  let last = -1;
  for (const at of [...keep].sort((x, y) => x - y)) {
    if (last >= 0 && at > last + 1) {
      out.push({ sign: " ", text: "…" });
    }
    out.push(diff[at]!);
    last = at;
  }
  return out;
}
