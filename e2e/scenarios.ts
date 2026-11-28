#!/usr/bin/env -S node --experimental-strip-types
/**
 * The app's scenarios (features/*.feature) against the e2e tests that cite them (issue #198,
 * ADR 0069). The feature files are the spec only; a test cites scenarios by id: a Playwright test
 * in its `tag` detail, an e2e/run.ts check in e2e/run-checks.ts.
 *
 *   pnpm scenarios check     every cited id exists, every test cites one; warns on uncovered
 *   pnpm scenarios report    the same, then the coverage of the last run, as Markdown
 *
 * Options: --features <dir> (default features/), --playwright <file> (a Playwright JSON report,
 * from a run or from `--list`; default for report e2e/results/playwright.json, else a fresh
 * `playwright test --list`), --run <file> (default for report e2e/results/run.json, else the
 * checks of e2e/run-checks.ts, not run), --out <file> (default e2e/results/scenarios.md).
 * The report is also appended to $GITHUB_STEP_SUMMARY when that is set.
 * --base <git ref>: also fail when a scenario id that features/ has at that ref is gone now. An id
 * never changes once merged (ADR 0069); a scenario is reworded under its id, not renamed.
 *
 * Exit 0 when the link holds (uncovered scenarios only warn), 1 when it does not, 2 on bad input.
 */
import { spawnSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { RUN_CHECKS, type RunResults } from "./run-checks.ts";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** The tag of a test that asserts no behaviour and so covers no scenario (the screenshot writer). */
export const TOOLING = "@tooling";
/** The tag of a `must fail:` test: it cites the scenario whose check it proves, and does not cover it. */
export const SELFTEST = "@selftest";
const MARKERS = new Set([TOOLING, SELFTEST]);
/** `@<capability>.<slug>`: lowercase words joined by hyphens, the capability the file's name. */
const ID = /^@([a-z][a-z0-9]*(?:-[a-z0-9]+)*)\.([a-z0-9]+(?:-[a-z0-9]+)*)$/;

export type Scenario = { id: string | null; name: string; file: string; line: number; tags: string[]; steps: number };
export type Feature = { name: string; file: string; capability: string; scenarios: Scenario[] };

/** A small Gherkin reader: enough for a spec nothing runs (A3). Tags before a scenario are its own. */
export function readFeature(file: string, text: string): { feature: Feature; problems: string[] } {
  const capability = basename(file).replace(/\.feature$/, "");
  const feature: Feature = { name: "", file, capability, scenarios: [] };
  const problems: string[] = [];
  let pending: string[] = [];
  let current: Scenario | null = null;
  text.split("\n").forEach((raw, i) => {
    const line = raw.trim();
    const at = `${file}:${i + 1}`;
    if (line === "" || line.startsWith("#")) {
      return;
    }
    if (line.startsWith("@")) {
      pending.push(...line.split(/\s+/).filter((t) => t !== ""));
      return;
    }
    const keyword = /^(Feature|Rule|Background|Scenario Outline|Scenario Template|Scenario|Example|Examples|Scenarios):\s*(.*)$/.exec(line);
    if (keyword) {
      const [, kw, name] = keyword as unknown as [string, string, string];
      if (kw === "Feature") {
        feature.name = name;
        pending = [];
      } else if (kw === "Scenario" || kw === "Example" || kw === "Scenario Outline" || kw === "Scenario Template") {
        const ids = pending.filter((t) => !MARKERS.has(t) && t.includes("."));
        current = { id: null, name, file, line: i + 1, tags: pending, steps: 0 };
        feature.scenarios.push(current);
        if (ids.length === 0) {
          problems.push(`${at}: scenario "${name}" has no id tag @${capability}.<slug>`);
        } else if (ids.length > 1) {
          problems.push(`${at}: scenario "${name}" has ${ids.length} id tags (${ids.join(", ")}); it takes one`);
        } else {
          const m = ID.exec(ids[0] as string);
          if (!m) {
            problems.push(`${at}: ${ids[0]} is not an id of the form @${capability}.<slug> (lowercase, hyphens)`);
          } else if (m[1] !== capability) {
            problems.push(`${at}: ${ids[0]} is in ${capability}.feature; its id starts @${capability}.`);
          } else {
            current.id = ids[0] as string;
          }
        }
        pending = [];
      } else {
        // Rule, Background, Examples: their tags are not a scenario's.
        if (kw !== "Examples" && kw !== "Scenarios") {
          current = null;
        }
        pending = [];
      }
      return;
    }
    if (/^(Given|When|Then|And|But|\*)\s/.test(line) && current) {
      current.steps += 1;
    }
  });
  if (!feature.name) {
    problems.push(`${file}: no Feature: line`);
  }
  for (const s of feature.scenarios) {
    if (s.steps === 0) {
      problems.push(`${s.file}:${s.line}: scenario "${s.name}" has no steps`);
    }
  }
  return { feature, problems };
}

export function readFeatures(dir: string): { features: Feature[]; problems: string[] } {
  const files = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".feature")).sort() : [];
  const features: Feature[] = [];
  const problems: string[] = files.length === 0 ? [`${dir}: no .feature files`] : [];
  for (const f of files) {
    const path = join(dir, f);
    const r = readFeature(relativeToRoot(path), readFileSync(path, "utf8"));
    features.push(r.feature);
    problems.push(...r.problems);
  }
  const seen = new Map<string, Scenario>();
  for (const s of features.flatMap((f) => f.scenarios)) {
    if (s.id === null) {
      continue;
    }
    const first = seen.get(s.id);
    if (first) {
      problems.push(`${s.file}:${s.line}: ${s.id} is already the id of "${first.name}" (${first.file}:${first.line})`);
    } else {
      seen.set(s.id, s);
    }
  }
  return { features, problems };
}

export type Status = "passed" | "failed" | "skipped" | "not run";
export type TestCase = { suite: "web" | "run"; title: string; file: string; line?: number; project?: string; tags: string[]; status: Status };

type PwSpec = { title: string; file: string; line: number; tags?: string[]; tests: { projectName: string; status: string; results: unknown[] }[] };
type PwSuite = { specs?: PwSpec[]; suites?: PwSuite[] };

/** Every (spec, project) of a Playwright JSON report. Playwright drops the `@` of a tag; it is put back. */
export function fromPlaywright(report: { suites?: PwSuite[] }): TestCase[] {
  const out: TestCase[] = [];
  const walk = (s: PwSuite) => {
    for (const spec of s.specs ?? []) {
      for (const t of spec.tests) {
        const status: Status =
          t.results.length === 0 ? "not run" : t.status === "expected" || t.status === "flaky" ? "passed" : t.status === "skipped" ? "skipped" : "failed";
        out.push({
          suite: "web",
          title: spec.title,
          file: `web/e2e/${spec.file}`,
          line: spec.line,
          project: t.projectName,
          tags: (spec.tags ?? []).map((tag) => (tag.startsWith("@") ? tag : `@${tag}`)),
          status,
        });
      }
    }
    for (const c of s.suites ?? []) {
      walk(c);
    }
  };
  for (const s of report.suites ?? []) {
    walk(s);
  }
  return out;
}

export function fromRun(results: RunResults): TestCase[] {
  return results.tests.map((t) => ({ suite: "run", title: t.title, file: t.file, tags: t.scenarios, status: t.status }));
}

/** The checks of e2e/run.ts, as a run that has not happened. */
export function runChecksNotRun(): TestCase[] {
  return RUN_CHECKS.map((c) => ({ suite: "run", title: c.title, file: "e2e/run.ts", tags: [...c.scenarios], status: "not run" }));
}

export const where = (t: TestCase) => `${t.file}${t.line ? `:${t.line}` : ""}${t.project ? ` [${t.project}]` : ""}`;

export type Validation = {
  scenarios: number;
  tests: number;
  tooling: number;
  selftests: number;
  invalid: string[];
  unknown: string[];
  untagged: string[];
  uncovered: Scenario[];
  ok: boolean;
};

/** What fails the check: a malformed or duplicate scenario, an id no scenario has, a test citing none. */
export function validate(features: Feature[], featureProblems: string[], tests: TestCase[]): Validation {
  const scenarios = features.flatMap((f) => f.scenarios);
  const ids = new Set(scenarios.flatMap((s) => (s.id ? [s.id] : [])));
  const unknown: string[] = [];
  const untagged: string[] = [];
  let tooling = 0;
  let selftests = 0;
  for (const t of tests) {
    tooling += t.tags.includes(TOOLING) ? 1 : 0;
    selftests += t.tags.includes(SELFTEST) ? 1 : 0;
    const cited = t.tags.filter((tag) => !MARKERS.has(tag));
    for (const tag of cited) {
      if (!ids.has(tag)) {
        unknown.push(`${where(t)} "${t.title}" cites ${tag}, which no scenario in features/ has`);
      }
    }
    if (cited.length === 0 && !t.tags.includes(TOOLING)) {
      untagged.push(`${where(t)} "${t.title}" cites no scenario (tag it with a scenario id, or ${TOOLING} if it asserts no behaviour)`);
    }
  }
  const cites = new Set(tests.filter(covers).flatMap((t) => t.tags));
  const uncovered = scenarios.filter((s) => s.id !== null && !cites.has(s.id));
  return {
    scenarios: scenarios.length,
    tests: tests.length,
    tooling,
    selftests,
    invalid: featureProblems,
    unknown,
    untagged,
    uncovered,
    ok: featureProblems.length === 0 && unknown.length === 0 && untagged.length === 0,
  };
}

/** A test that counts as covering what it cites: not tooling, not a selftest. */
const covers = (t: TestCase) => !t.tags.includes(TOOLING) && !t.tags.includes(SELFTEST);

export type State = "covered" | "partly" | "not run" | "not covered";
export type Coverage = {
  scenario: Scenario;
  capability: string;
  state: State;
  tests: TestCase[];
  selftests: TestCase[];
  passed: number;
  failed: number;
  skipped: number;
  notRun: number;
};

/** The first rule that holds (ADR 0069): not covered, covered and passing, not run, partly covered. */
export function coverage(features: Feature[], tests: TestCase[]): Coverage[] {
  return features.flatMap((f) =>
    f.scenarios
      .filter((s) => s.id !== null)
      .map((s) => {
        const citing = tests.filter((t) => t.tags.includes(s.id as string));
        const counting = citing.filter(covers);
        const n = (st: Status) => counting.filter((t) => t.status === st).length;
        const [passed, failed, skipped, notRun] = [n("passed"), n("failed"), n("skipped"), n("not run")];
        const state: State =
          counting.length === 0 ? "not covered" : passed === counting.length ? "covered" : notRun === counting.length ? "not run" : "partly";
        return { scenario: s, capability: f.capability, state, tests: counting, selftests: citing.filter((t) => t.tags.includes(SELFTEST)), passed, failed, skipped, notRun };
      }),
  );
}

const MARK: Record<State, string> = { covered: "✅", partly: "🟡", "not run": "⏸", "not covered": "⬜" };

function stateCell(r: Coverage): string {
  switch (r.state) {
    case "not covered":
      return `${MARK[r.state]} not covered`;
    case "not run":
      return `${MARK[r.state]} not run (${r.notRun})`;
    case "covered":
      return `${MARK[r.state]} ${r.passed} of ${r.tests.length} passed`;
    case "partly": {
      const rest = [r.failed && `${r.failed} failed`, r.skipped && `${r.skipped} skipped`, r.notRun && `${r.notRun} not run`].filter(Boolean).join(", ");
      return `${MARK[r.state]} ${r.passed} of ${r.tests.length} passed; ${rest}`;
    }
  }
}
const cell = (s: string) => s.replace(/\|/g, "\\|").replace(/\n/g, " ");

export function renderMarkdown(rows: Coverage[], v: Validation, tests: TestCase[], sources: string[] = []): string {
  const count = (s: State) => rows.filter((r) => r.state === s).length;
  const suites = (["web", "run"] as const).map((s) => {
    const of = tests.filter((t) => t.suite === s);
    const ran = of.filter((t) => t.status !== "not run").length;
    return `${s === "web" ? "web/e2e" : "e2e/run.ts"} ${ran}/${of.length} run`;
  });
  const out: string[] = [
    "## Scenario coverage",
    "",
    `Checked ${v.scenarios} scenarios and ${v.tests} tests (${suites.join(", ")}; ${v.selftests} selftests, ${v.tooling} tooling).`,
    "",
    `${MARK.covered} ${count("covered")} covered and passing · ${MARK.partly} ${count("partly")} partly covered · ${MARK["not run"]} ${count("not run")} not run · ${MARK["not covered"]} ${count("not covered")} not covered`,
    "",
    ...sources.map((s) => `- ${s}`),
    ...(sources.length ? [""] : []),
  ];
  if (!v.ok) {
    out.push("### The check fails", "");
    for (const p of [...v.invalid, ...v.unknown, ...v.untagged]) {
      out.push(`- ${p}`);
    }
    out.push("");
  }
  out.push("| Scenario | Coverage | Tests |", "| --- | --- | --- |");
  let capability = "";
  for (const r of rows) {
    if (r.capability !== capability) {
      capability = r.capability;
      out.push(`| **${capability}** | | |`);
    }
    const state = stateCell(r);
    const tested = r.tests.map((t) => `${cell(t.title)} (\`${where(t)}\`, ${t.status})`).join("<br>");
    out.push(`| \`${r.scenario.id}\` ${cell(r.scenario.name)} | ${state} | ${tested || "—"} |`);
  }
  const gaps = rows.filter((r) => r.state === "not covered");
  out.push("", `### Not covered (${gaps.length})`, "");
  out.push(...(gaps.length ? gaps.map((r) => `- \`${r.scenario.id}\` ${r.scenario.name} (${r.scenario.file}:${r.scenario.line})`) : ["None."]));
  for (const [tag, name] of [
    [SELFTEST, "Selftests"],
    [TOOLING, "Tooling"],
  ] as const) {
    const apart = tests.filter((t) => t.tags.includes(tag));
    if (apart.length) {
      out.push("", `### ${name}, covering no scenario (${apart.length})`, "");
      out.push(...apart.map((t) => `- ${cell(t.title)} (\`${where(t)}\`, ${t.status}${tag === SELFTEST ? `; proves ${t.tags.filter((x) => !MARKERS.has(x)).join(", ")}` : ""})`));
    }
  }
  return `${out.join("\n")}\n`;
}

/** The scenario ids in feature files' text, wherever they sit: enough to say which ids a ref had. */
export function idsIn(texts: string[]): string[] {
  const ids = new Set<string>();
  for (const text of texts) {
    for (const line of text.split("\n")) {
      const t = line.trim();
      if (t.startsWith("@")) {
        for (const tag of t.split(/\s+/)) {
          if (ID.test(tag)) {
            ids.add(tag);
          }
        }
      }
    }
  }
  return [...ids].sort();
}

/** The ids the base had that no scenario has now (ADR 0069: ids never change once merged). */
export function goneSince(baseIds: string[], features: Feature[]): string[] {
  const now = new Set(features.flatMap((f) => f.scenarios.flatMap((s) => (s.id ? [s.id] : []))));
  return baseIds.filter((id) => !now.has(id));
}

/** The text of every feature file at a git ref. */
function featuresAt(ref: string): string[] {
  const git = (...args: string[]) => {
    const r = spawnSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    if (r.status !== 0) {
      throw new Error(`git ${args.join(" ")} failed: ${r.stderr.trim()}`);
    }
    return r.stdout;
  };
  const files = git("ls-tree", "-r", "--name-only", ref, "features/").split("\n").filter((f) => f.endsWith(".feature"));
  return files.map((f) => git("show", `${ref}:${f}`));
}

function relativeToRoot(p: string): string {
  const abs = resolve(p);
  return abs.startsWith(`${ROOT}/`) ? abs.slice(ROOT.length + 1) : p;
}

function listPlaywright(): { suites?: PwSuite[] } {
  const bin = join(ROOT, "web/node_modules/.bin/playwright");
  const r = spawnSync(bin, ["test", "--list", "--reporter=json"], { cwd: join(ROOT, "web"), encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (r.status !== 0) {
    throw new Error(`playwright test --list failed (${r.status}): ${r.stderr || r.stdout}`);
  }
  return JSON.parse(r.stdout) as { suites?: PwSuite[] };
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

export function main(argv: string[], env: NodeJS.ProcessEnv = process.env): number {
  const [command, ...rest] = argv;
  const opt = (name: string) => {
    const i = rest.indexOf(`--${name}`);
    return i >= 0 ? rest[i + 1] : undefined;
  };
  if (command !== "check" && command !== "report") {
    console.error("usage: scenarios.ts check|report [--features <dir>] [--playwright <file>] [--run <file>] [--out <file>] [--base <git ref>]");
    return 2;
  }
  const report = command === "report";
  let tests: TestCase[];
  let features: Feature[];
  let featureProblems: string[];
  let baseIds: string[] | null = null;
  const sources: string[] = [];
  try {
    ({ features, problems: featureProblems } = readFeatures(opt("features") ?? join(ROOT, "features")));
    const base = opt("base");
    baseIds = base ? idsIn(featuresAt(base)) : null;
    const pwFile = opt("playwright") ?? (report && existsSync(join(ROOT, "e2e/results/playwright.json")) ? join(ROOT, "e2e/results/playwright.json") : undefined);
    const runFile = opt("run") ?? (report && existsSync(join(ROOT, "e2e/results/run.json")) ? join(ROOT, "e2e/results/run.json") : undefined);
    tests = [...fromPlaywright(pwFile ? readJson(pwFile) : listPlaywright()), ...(runFile ? fromRun(readJson<RunResults>(runFile)) : runChecksNotRun())];
    sources.push(pwFile ? `web/e2e: results from ${relativeToRoot(pwFile)}` : "web/e2e: no results file; tests from `playwright test --list`, not run");
    sources.push(runFile ? `e2e/run.ts: results from ${relativeToRoot(runFile)}` : "e2e/run.ts: no results file; checks from e2e/run-checks.ts, not run");
  } catch (e) {
    console.error(`scenarios: ${e instanceof Error ? e.message : String(e)}`);
    return 2;
  }
  const v = validate(features, featureProblems, tests);
  const gone = baseIds ? goneSince(baseIds, features) : [];
  for (const p of [...v.invalid, ...v.unknown, ...v.untagged]) {
    console.log(`error: ${p}`);
  }
  for (const id of gone) {
    console.log(`error: ${id} is in features/ at ${opt("base")} and gone now; an id never changes once merged (ADR 0069)`);
  }
  const ok = v.ok && gone.length === 0;
  for (const s of v.uncovered) {
    console.log(`warning: ${s.id} "${s.name}" (${s.file}:${s.line}) is cited by no test`);
    if (env["GITHUB_ACTIONS"] === "true") {
      console.log(`::warning file=${s.file},line=${s.line},title=Scenario not covered::${s.id} is cited by no e2e test`);
    }
  }
  if (report) {
    for (const s of sources) {
      console.log(s);
    }
    const md = renderMarkdown(coverage(features, tests), v, tests, sources);
    const out = opt("out") ?? join(ROOT, "e2e/results/scenarios.md");
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, md);
    console.log(`wrote ${relativeToRoot(out)}`);
    if (env["GITHUB_STEP_SUMMARY"]) {
      appendFileSync(env["GITHUB_STEP_SUMMARY"], md);
    }
  }
  console.log(
    JSON.stringify({
      event: "rubato.scenarios.check",
      command,
      scenarios: v.scenarios,
      tests: v.tests,
      tooling: v.tooling,
      selftests: v.selftests,
      invalid: v.invalid.length,
      unknown: v.unknown.length,
      untagged: v.untagged.length,
      uncovered: v.uncovered.length,
      ...(baseIds ? { base_ids: baseIds.length, gone: gone.length } : {}),
      outcome: ok ? "pass" : "fail",
    }),
  );
  console.log(`checked ${v.scenarios} scenarios and ${v.tests} tests: ${ok ? "ok" : "failed"}, ${v.uncovered.length} not covered${baseIds ? `; ${baseIds.length} ids of ${opt("base")} checked, ${gone.length} gone` : ""}`);
  return ok ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
