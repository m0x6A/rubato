#!/usr/bin/env -S node --experimental-strip-types
/**
 * rubato plan check.
 *
 *   plan-check --map <system-map.yml> --schema-dir <dir> [--plans <glob>]...
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
  map: string;
  schemaDir: string;
  plans: string[];
}

export function parseArgs(argv: string[]): Args {
  const here = dirname(fileURLToPath(import.meta.url));
  const args: Args = { map: "", schemaDir: resolve(here, "../../../schema"), plans: [] };
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
      args.map = next();
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
  try {
    map = loadMap(args.map, args.schemaDir);
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
    const r = checkPlanFile(f, map, args.schemaDir);
    ids += r.idsChecked;
    flows += r.flowsChecked;
    models += r.dataModelsChecked;
    legacy += r.legacyKeys.length;
    legacyPlans += r.legacyKeys.length > 0 ? 1 : 0;
    if (!r.ok) {
      failed++;
    }
    console.log(JSON.stringify(logLine(r)));
  }
  console.log(`plan-check: checked ${files.length} plan(s), ${ids} id(s), ${flows} flow(s) and ${models} data model(s) against ${map.nodes.length} nodes and ${(map.seams ?? []).length} seams in ${args.map}; ${legacy} legacy front-matter key(s) in ${legacyPlans} plan(s), not failed; ${failed} failed`);
  return failed === 0 ? 0 : 1;
}

const invokedDirectly = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  process.exitCode = main(process.argv.slice(2));
}

export const schemaDirDefault = join(dirname(fileURLToPath(import.meta.url)), "../../../schema");
