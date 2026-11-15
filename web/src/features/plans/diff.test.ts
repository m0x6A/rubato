import { describe, expect, it } from "vitest";
import { bundleOrder, diffBundles, diffLines, hunks, splitLines } from "./diff.ts";

/** The diff as one string per line: `+` added, `-` removed, a space unchanged. */
const show = (a: string, b: string, maxCells?: number) =>
  diffLines(splitLines(a), splitLines(b), maxCells).lines.map((l) => `${l.op === "add" ? "+" : l.op === "del" ? "-" : " "}${l.text}`);

describe("the line diff (LCS)", () => {
  it("leaves equal texts unchanged", () => {
    expect(show("a\nb\n", "a\nb\n")).toEqual([" a", " b"]);
  });

  it("finds an inserted, a removed and a replaced line, removal before addition", () => {
    expect(show("a\nc\n", "a\nb\nc\n")).toEqual([" a", "+b", " c"]);
    expect(show("a\nb\nc\n", "a\nc\n")).toEqual([" a", "-b", " c"]);
    expect(show("a\nold\nc\n", "a\nnew\nc\n")).toEqual([" a", "-old", "+new", " c"]);
  });

  it("keeps the longest common subsequence, not just a common prefix", () => {
    expect(show("a\nb\nc\nd\n", "a\nc\nd\ne\n")).toEqual([" a", "-b", " c", " d", "+e"]);
    expect(show("x\na\nb\n", "a\nb\ny\n")).toEqual(["-x", " a", " b", "+y"]);
  });

  it("an empty side is all added or all removed", () => {
    expect(show("", "a\nb\n")).toEqual(["+a", "+b"]);
    expect(show("a\nb\n", "")).toEqual(["-a", "-b"]);
    expect(show("", "")).toEqual([]);
  });

  it("splits on LF and CRLF, and a final newline is not an extra empty line", () => {
    expect(splitLines("a\r\nb\r\n")).toEqual(["a", "b"]);
    expect(splitLines("a\nb")).toEqual(["a", "b"]);
    expect(splitLines("a\n\n")).toEqual(["a", ""]);
  });

  it("past the cell cap the changed middle is shown removed then added, and says it is not exact", () => {
    const r = diffLines(splitLines("head\na\nb\nc\ntail\n"), splitLines("head\nb\nx\ntail\n"), 4);
    expect(r.exact).toBe(false);
    expect(r.lines.map((l) => `${l.op}:${l.text}`)).toEqual(["same:head", "del:a", "del:b", "del:c", "add:b", "add:x", "same:tail"]);
    // Under the cap the same texts diff exactly.
    expect(diffLines(splitLines("head\na\nb\nc\ntail\n"), splitLines("head\nb\nx\ntail\n")).exact).toBe(true);
  });
});

describe("hunks: changes with context, unchanged runs folded", () => {
  it("keeps `context` lines around each change and counts the folded ones", () => {
    const lines = diffLines(splitLines("1\n2\n3\n4\n5\n6\n7\n8\n9\n"), splitLines("1\n2\n3\n4\nfive\n6\n7\n8\n9\n")).lines;
    const h = hunks(lines, 1).map((x) => ("gap" in x ? `…${x.gap}` : `${x.op}:${x.text}`));
    expect(h).toEqual(["…3", "same:4", "del:5", "add:five", "same:6", "…3"]);
  });

  it("does not fold a run shorter than what it would hide", () => {
    const lines = diffLines(splitLines("a\nb\nc\n"), splitLines("A\nb\nC\n")).lines;
    expect(hunks(lines, 1).some((x) => "gap" in x)).toBe(false);
  });
});

describe("a bundle's files: the plan first, then the rest by path", () => {
  const plan = { path: "docs/plans/122.md", text: "# Plan\nold\nkept\n" };

  it("orders the plan (the bundle's first file) first and the rest by path", () => {
    const ordered = bundleOrder([plan, { path: "docs/plans/122-x/b.md", text: "" }, { path: "docs/decisions/0053-x.md", text: "" }, { path: "docs/plans/122-x/a.md", text: "" }]);
    expect(ordered.map((f) => f.path)).toEqual(["docs/plans/122.md", "docs/decisions/0053-x.md", "docs/plans/122-x/a.md", "docs/plans/122-x/b.md"]);
  });

  it("reports added, removed and changed files with their counts, and leaves unchanged ones out", () => {
    const before = [plan, { path: "docs/plans/122-x/design.md", text: "gone\n" }, { path: "docs/plans/122-x/same.md", text: "same\n" }];
    const after = [{ ...plan, text: "# Plan\nnew\nkept\nmore\n" }, { path: "docs/plans/122-x/same.md", text: "same\n" }, { path: "docs/decisions/0053-x.md", text: "adr\nbody\n" }];
    const changes = diffBundles(before, after);
    expect(changes.map((c) => [c.path, c.status, c.added, c.removed])).toEqual([
      ["docs/plans/122.md", "changed", 2, 1],
      ["docs/decisions/0053-x.md", "added", 2, 0],
      ["docs/plans/122-x/design.md", "removed", 0, 1],
    ]);
  });

  it("two equal bundles have no changes", () => {
    expect(diffBundles([plan], [{ ...plan }])).toEqual([]);
  });
});
