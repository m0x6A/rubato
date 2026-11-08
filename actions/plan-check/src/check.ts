/**
 * The plan check: unknown ids, seams without an O-row, schema validity. One log line per plan,
 * named `rubato.plan.check`, carrying the `rubato.*` attributes from `rubato-protocol::otel`.
 */
import { readFileSync } from "node:fs";
import { mapIds, type SystemMap } from "./map.ts";
import { parsePlan, PlanParseError, type Plan } from "./parse-plan.ts";
import { validator } from "./schema.ts";

export interface PlanCheckResult {
  path: string;
  plan: Plan | null;
  parseError: string | null;
  schemaErrors: string[];
  unknownIds: string[];
  seamsWithoutORow: string[];
  idsChecked: number;
  ok: boolean;
}

export function referencedIds(plan: Plan): string[] {
  const ids = new Set<string>();
  for (const p of plan.affected_parts) {
    ids.add(p.node);
    for (const s of p.seams ?? []) {
      ids.add(s);
    }
  }
  for (const o of plan.o_rows ?? []) {
    ids.add(o.seam);
    ids.add(o.owner);
  }
  return [...ids].sort();
}

export function seamsWithoutORow(plan: Plan): string[] {
  const covered = new Set((plan.o_rows ?? []).map((o) => o.seam));
  const missing = new Set<string>();
  for (const p of plan.affected_parts) {
    for (const s of p.seams ?? []) {
      if (!covered.has(s)) {
        missing.add(s);
      }
    }
  }
  return [...missing].sort();
}

export function checkPlanText(text: string, path: string, map: SystemMap, schemaDir: string): PlanCheckResult {
  const result: PlanCheckResult = {
    path,
    plan: null,
    parseError: null,
    schemaErrors: [],
    unknownIds: [],
    seamsWithoutORow: [],
    idsChecked: 0,
    ok: false,
  };
  let plan: Plan;
  try {
    plan = parsePlan(text, path);
  } catch (e) {
    result.parseError = e instanceof PlanParseError ? e.message : String(e);
    return result;
  }
  result.plan = plan;
  const validate = validator(schemaDir, "plan.schema.json");
  if (!validate(plan)) {
    result.schemaErrors = (validate.errors ?? []).map((e) => `${e.instancePath || "/"} ${e.message ?? ""}`.trim());
  }
  const known = mapIds(map);
  const ids = referencedIds(plan);
  result.idsChecked = ids.length;
  result.unknownIds = ids.filter((id) => !known.has(id));
  result.seamsWithoutORow = seamsWithoutORow(plan);
  result.ok = result.schemaErrors.length === 0 && result.unknownIds.length === 0 && result.seamsWithoutORow.length === 0;
  return result;
}

export function checkPlanFile(path: string, map: SystemMap, schemaDir: string): PlanCheckResult {
  return checkPlanText(readFileSync(path, "utf8"), path, map, schemaDir);
}

/** The structured log line. Attribute names match `rubato-protocol::otel`. */
export function logLine(r: PlanCheckResult): Record<string, unknown> {
  return {
    event: "rubato.plan.check",
    "rubato.repo": r.plan?.issue.repo ?? null,
    "rubato.issue": r.plan ? `${r.plan.issue.repo}#${r.plan.issue.number}` : null,
    "rubato.plan.path": r.path,
    "rubato.plan.unknown_ids": r.unknownIds,
    "rubato.plan.o_rows_missing": r.seamsWithoutORow,
    ids_checked: r.idsChecked,
    schema_errors: r.schemaErrors,
    parse_error: r.parseError,
    ok: r.ok,
  };
}
