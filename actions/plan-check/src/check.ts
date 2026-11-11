/**
 * The plan check: unknown ids, seams without an O-row, schema validity. One log line per plan,
 * named `rubato.plan.check`, carrying the `rubato.*` attributes from `rubato-protocol::otel`.
 */
import { readFileSync } from "node:fs";
import { mapIds, seamJoins, type SystemMap } from "./map.ts";
import { parsePlan, PlanParseError, type Plan, type PlanFlow } from "./parse-plan.ts";
import { validator } from "./schema.ts";

export interface PlanCheckResult {
  path: string;
  plan: Plan | null;
  parseError: string | null;
  schemaErrors: string[];
  unknownIds: string[];
  seamsWithoutORow: string[];
  idsChecked: number;
  /** Rows of the plan's Flows table that were checked (ADR 0021). */
  flowsChecked: number;
  /** Flows whose seam does not join their two parts, as `from → to over seam`. */
  flowsOffSeam: string[];
  ok: boolean;
}

/** Flows whose seam the map has but which does not join the flow's two parts. */
export function flowsOffSeam(plan: Plan, map: SystemMap): PlanFlow[] {
  return (plan.flows ?? []).filter((f) => {
    const seam = (map.seams ?? []).find((s) => s.id === f.seam);
    return seam !== undefined && !seamJoins(seam, f.from, f.to);
  });
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
  for (const f of plan.flows ?? []) {
    ids.add(f.from);
    ids.add(f.to);
    ids.add(f.seam);
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
    flowsChecked: 0,
    flowsOffSeam: [],
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
  result.flowsChecked = plan.flows?.length ?? 0;
  result.flowsOffSeam = flowsOffSeam(plan, map).map((f) => `${f.from} → ${f.to} over ${f.seam}`);
  result.ok = result.schemaErrors.length === 0 && result.unknownIds.length === 0 && result.seamsWithoutORow.length === 0 && result.flowsOffSeam.length === 0;
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
    flows_checked: r.flowsChecked,
    flows_off_seam: r.flowsOffSeam,
    schema_errors: r.schemaErrors,
    parse_error: r.parseError,
    ok: r.ok,
  };
}
