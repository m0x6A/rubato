/**
 * The web's vertical-slice rule (ADR 0015, design D3/D9): a feature names another feature only
 * through that feature's `index.ts`, otherwise only `shared/`, `protocol.ts`, its own files and
 * external packages (test files may also use `web/test/`); the feature graph is acyclic; the
 * folders equal the list in AGENTS.md's `<!-- slices: web -->` block.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, posix, resolve } from "node:path";
import { describe, expect, it } from "vitest";

export interface SourceFile {
  /** Path relative to `web/src/features`, posix separators, e.g. `issues/IssuePanel.tsx`. */
  path: string;
  source: string;
}

export interface CheckResult {
  features: string[];
  imports: number;
  edges: [string, string][];
  errors: string[];
}

const IMPORT_PATTERNS = [
  /\b(?:import|export)\s+(?:type\s+)?[^'"`;]*?\bfrom\s*["']([^"']+)["']/g,
  /\bimport\s*["']([^"']+)["']/g,
  /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
];

/** Every module specifier a source file imports or re-exports, in order of appearance. */
export function importsOf(source: string): string[] {
  const found: { at: number; spec: string }[] = [];
  for (const re of IMPORT_PATTERNS) {
    for (const m of source.matchAll(re)) {
      found.push({ at: m.index ?? 0, spec: m[1] as string });
    }
  }
  return found.sort((a, b) => a.at - b.at).map((f) => f.spec);
}

const isTest = (path: string) => /\.test\.tsx?$/.test(path);
const INDEX = /^(index|index\.ts|index\.tsx)?$/;

/** Pure: checks the slice rule over the given files. Paths outside a feature folder are ignored. */
export function checkFeatures(files: SourceFile[]): CheckResult {
  const errors: string[] = [];
  const edges = new Map<string, Set<string>>();
  const features = [...new Set(files.filter((f) => f.path.includes("/")).map((f) => f.path.split("/")[0] as string))].sort();
  let imports = 0;

  for (const file of files) {
    if (!file.path.includes("/")) {
      continue; // this checker and other files at the features root are not a feature
    }
    const own = file.path.split("/")[0] as string;
    for (const spec of importsOf(file.source)) {
      imports += 1;
      if (!spec.startsWith(".")) {
        continue; // external package or node builtin
      }
      // Resolve against `features/<file dir>`; a result starting with `..` left `src/features`.
      const target = posix.normalize(posix.join(posix.dirname(file.path), spec));
      const where = `${file.path} imports "${spec}"`;
      if (target.startsWith("../")) {
        const fromSrc = target.slice(3); // relative to web/src
        if (fromSrc.startsWith("shared/") || fromSrc === "protocol.ts" || fromSrc === "protocol") {
          continue;
        }
        if (fromSrc.startsWith("../test/") && isTest(file.path)) {
          continue;
        }
        errors.push(`${where}: a feature may only import shared/, protocol.ts or another feature's index.ts`);
        continue;
      }
      const [feature, ...rest] = target.split("/");
      if (feature === own) {
        continue;
      }
      if (rest.length > 1 || !INDEX.test(rest[0] ?? "")) {
        errors.push(`${where}: deep import into feature "${feature}"; use ../${feature}/index.ts`);
        continue;
      }
      if (!edges.has(own)) {
        edges.set(own, new Set());
      }
      edges.get(own)?.add(feature as string);
    }
  }

  // Cycle detection: depth-first, reporting each cycle once by its path.
  const state = new Map<string, "visiting" | "done">();
  const visit = (node: string, path: string[]) => {
    if (state.get(node) === "done") {
      return;
    }
    if (state.get(node) === "visiting") {
      errors.push(`cycle: ${[...path.slice(path.indexOf(node)), node].join(" -> ")}`);
      return;
    }
    state.set(node, "visiting");
    for (const next of [...(edges.get(node) ?? [])].sort()) {
      visit(next, [...path, node]);
    }
    state.set(node, "done");
  };
  for (const node of [...edges.keys()].sort()) {
    visit(node, []);
  }

  const edgeList = [...edges].flatMap(([from, tos]) => [...tos].map((to) => [from, to] as [string, string])).sort();
  return { features, imports, edges: edgeList, errors };
}

/** The first word of each line of the ```text fence inside AGENTS.md's `<!-- slices: web -->` block. */
export function declaredFeatures(agentsMd: string): string[] {
  const block = /<!-- slices: web -->([\s\S]*?)<!-- \/slices -->/.exec(agentsMd)?.[1];
  const fence = block ? /```text\n([\s\S]*?)```/.exec(block)?.[1] : undefined;
  if (!fence) {
    throw new Error("AGENTS.md has no ```text list inside <!-- slices: web -->");
  }
  return fence
    .split("\n")
    .map((l) => l.trim().split(/\s+/)[0] ?? "")
    .filter(Boolean)
    .sort();
}

const here = import.meta.dirname;

function readTree(dir: string, prefix = ""): SourceFile[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    const rel = prefix ? `${prefix}/${name}` : name;
    if (statSync(full).isDirectory()) {
      return readTree(full, rel);
    }
    return /\.tsx?$/.test(name) ? [{ path: rel, source: readFileSync(full, "utf8") }] : [];
  });
}

describe("web features (ADR 0015)", () => {
  const files = readTree(here);
  const result = checkFeatures(files);

  it("cross-feature imports go through index.ts, nothing else leaves shared/ and protocol.ts, no cycles", () => {
    console.log(`checked ${result.features.length} features, ${result.imports} imports (edges: ${result.edges.map(([a, b]) => `${a}->${b}`).join(", ") || "none"})`);
    expect(result.errors).toEqual([]);
    expect(result.imports).toBeGreaterThan(0);
  });

  it("every feature folder has an index.ts", () => {
    const folders = readdirSync(here).filter((n) => statSync(join(here, n)).isDirectory());
    expect(folders.filter((f) => !files.some((s) => s.path === `${f}/index.ts`))).toEqual([]);
  });

  it("the feature folders equal the <!-- slices: web --> list in AGENTS.md", () => {
    const folders = readdirSync(here)
      .filter((n) => statSync(join(here, n)).isDirectory())
      .sort();
    expect(folders).toEqual(declaredFeatures(readFileSync(resolve(here, "../../../AGENTS.md"), "utf8")));
  });

  it("must fail: a deep import into another feature's inner file", () => {
    const r = checkFeatures([
      { path: "issues/IssuePanel.tsx", source: `import { launch } from "../launch/api.ts";\n` },
      { path: "launch/index.ts", source: `export { launch } from "./api.ts";\n` },
      { path: "launch/api.ts", source: `import { request } from "../../shared/api.ts";\n` },
    ]);
    expect(r.errors).toHaveLength(1);
    expect(r.errors[0]).toContain("deep import into feature \"launch\"");
  });

  it("must fail: a cycle between two features", () => {
    const r = checkFeatures([
      { path: "pins/index.ts", source: `export { PinDialog } from "./PinDialog.tsx";\nimport type { X } from "../issues/index.ts";\n` },
      { path: "pins/PinDialog.tsx", source: `export const PinDialog = 1;\n` },
      { path: "issues/index.ts", source: `import {\n  PinDialog,\n} from "../pins";\n` },
    ]);
    expect(r.edges).toEqual([
      ["issues", "pins"],
      ["pins", "issues"],
    ]);
    expect(r.errors).toEqual(["cycle: issues -> pins -> issues"]);
  });
});
