/**
 * The plan-check code is the one implementation of map loading and plan checking. Inside the
 * rubato checkout it is imported from `actions/plan-check/src`; an installed copy of the skill
 * carries it under `vendor/plan-check` (see `install.ts`). Schemas resolve the same way.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
export const skillRoot = resolve(here, "..", "..");

function firstExisting(candidates: string[], what: string): string {
  for (const c of candidates) {
    if (existsSync(c)) {
      return c;
    }
  }
  throw new Error(`cannot find ${what}; looked in ${candidates.join(", ")}`);
}

export function planCheckSrc(): string {
  return firstExisting([resolve(skillRoot, "vendor", "plan-check"), resolve(skillRoot, "..", "..", "actions", "plan-check", "src")], "plan-check sources");
}

export function schemaDir(): string {
  return firstExisting([resolve(skillRoot, "schema"), resolve(skillRoot, "..", "..", "schema")], "schema directory");
}

type CheckModule = typeof import("../../../../actions/plan-check/src/check.ts");
type MapModule = typeof import("../../../../actions/plan-check/src/map.ts");
type ParseModule = typeof import("../../../../actions/plan-check/src/parse-plan.ts");

export async function planCheck(): Promise<{ check: CheckModule; map: MapModule; parse: ParseModule }> {
  const src = planCheckSrc();
  const load = (f: string) => import(pathToFileURL(resolve(src, f)).href);
  const [check, map, parse] = await Promise.all([load("check.ts"), load("map.ts"), load("parse-plan.ts")]);
  return { check: check as CheckModule, map: map as MapModule, parse: parse as ParseModule };
}
