#!/usr/bin/env -S node --experimental-strip-types
/**
 * rubato plan check.
 *
 *   plan-check --map <system-map.yml> [--map <another.yml>]... [--repo owner/name] --schema-dir <dir> [--plans <glob>]...
 *
 * With several maps (one per workspace, ADR 0045), a plan passes when it fits any map that names
 * `--repo` (default `$GITHUB_REPOSITORY`), and its line says how many were checked and which fit.
 * With one map the output is as it always was.
 *
 * Exit 0 when every plan passes, 1 otherwise, 2 on bad usage or an invalid map. Prints one JSON
 * line per plan and a summary line, so every check reports how many things it checked.
 */
import { globSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkPlanFile, logLine } from "./check.ts";
import { loadMap, MapError } from "./map.ts";

interface Args {
  /** The first `--map`, as before workspaces. */
  map: string;
  maps: string[];
  repo: string | undefined;
  schemaDir: string;
  plans: string[];
}

export function parseArgs(argv: string[]): Args {
  const here = dirname(fileURLToPath(import.meta.url));
  const args: Args = { map: "", maps: [], repo: process.env["GITHUB_REPOSITORY"], schemaDir: resolve(here, "../../../schema"), plans: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) {
        throw new Error(`${a} needs a value`);
      }
      return v;
    };
    if (a === "--map") {
      args.maps.push(next());
      args.map ||= args.maps[0] as string;
    } else if (a === "--repo") {
      args.repo = next();
    } else if (a === "--schema-dir") {
      args.schemaDir = next();
    } else if (a === "--plans") {
      args.plans.push(next());
    } else {
      throw new Error(`unknown argument ${a}`);
    }
  }
  if (!args.map) {
    throw new Error("--map is required");
  }
  if (!args.plans.length) {
    args.plans.push("docs/plans/*.md");
  }
  return args;
}

export function main(argv: string[]): number {
  let args: Args;
  try {
    args = parseArgs(argv);
  } catch (e) {
    console.error(String(e instanceof Error ? e.message : e));
    return 2;
  }
  let map;
  const others: { file: string; map: ReturnType<typeof loadMap> }[] = [];
  try {
    map = loadMap(args.map, args.schemaDir);
    for (const file of args.maps.slice(1)) {
      others.push({ file, map: loadMap(file, args.schemaDir) });
    }
  } catch (e) {
    if (e instanceof MapError) {
      console.error(`${e.message}:\n  ${e.problems.join("\n  ")}`);
    } else {
      console.error(String(e));
    }
    return 2;
  }
  const files = [...new Set(args.plans.flatMap((g) => globSync(g)))].sort();
  let failed = 0;
  let ids = 0;
  let flows = 0;
  let models = 0;
  let legacy = 0;
  let legacyPlans = 0;
  for (const f of files) {
    let r = checkPlanFile(f, map, args.schemaDir);
    let extra: Record<string, unknown> = {};
    if (others.length) {
      // Several maps: those naming the repo; a plan passes when it fits any of them.
      const names = (m: typeof map) => !args.repo || (m.repos ?? []).some((x) => x.name.toLowerCase() === args.repo?.toLowerCase()) || m.nodes.some((n) => (n.repo ?? "").toLowerCase() === args.repo?.toLowerCase());
      const candidates = [{ file: args.map, map }, ...others].filter((m) => names(m.map));
      const results = candidates.map((m) => ({ file: m.file, r: checkPlanFile(f, m.map, args.schemaDir) }));
      const fit = results.filter((x) => x.r.ok);
      r = (fit[0] ?? results[0])?.r ?? r;
      extra = { maps_checked: results.length, fits: fit.map((x) => x.file) };
    }
    ids += r.idsChecked;
    flows += r.flowsChecked;
    models += r.dataModelsChecked;
    legacy += r.legacyKeys.length;
    legacyPlans += r.legacyKeys.length > 0 ? 1 : 0;
    if (!r.ok) {
      failed++;
    }
    console.log(JSON.stringify({ ...logLine(r), ...extra }));
  }
  console.log(`plan-check: checked ${files.length} plan(s), ${ids} id(s), ${flows} flow(s) and ${models} data model(s) against ${map.nodes.length} nodes and ${(map.seams ?? []).length} seams in ${args.map}${others.length ? ` and ${others.length} other map(s)` : ""}; ${legacy} legacy front-matter key(s) in ${legacyPlans} plan(s), not failed; ${failed} failed`);
  return failed === 0 ? 0 : 1;
}

const invokedDirectly = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  process.exitCode = main(process.argv.slice(2));
}

export const schemaDirDefault = join(dirname(fileURLToPath(import.meta.url)), "../../../schema");
