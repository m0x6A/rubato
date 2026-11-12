/**
 * The plan check: unknown ids, seams without an O-row, schema validity. Legacy front-matter keys
 * are counted and reported, never failed (ADR 0027). One log line per plan,
 * named `rubato.plan.check`, carrying the `rubato.*` attributes from `rubato-protocol::otel`.
 */
import { readFileSync } from "node:fs";
import { mapIds, seamJoins, type SystemMap } from "./map.ts";
import { legacyKeys, parsePlan, PlanParseError, type DataModel, type Plan, type PlanFlow } from "./parse-plan.ts";
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
  /** Rows of the plan's Data models table that were checked (ADR 0031). */
  dataModelsChecked: number;
  /** Models whose part is not a row of Affected parts, as `model on part`. */
  modelsOffPart: string[];
  /** Models crossing a seam no row of Affected parts touches, as `model over seam`. */
  modelsOffSeam: string[];
  /** Front-matter keys ADR 0027 moved to issue comments (review, approval, track, status). Reported, never failed. */
  legacyKeys: string[];
  ok: boolean;
}

/** Flows whose seam the map has but which does not join the flow's two parts. */
export function flowsOffSeam(plan: Plan, map: SystemMap): PlanFlow[] {
  return (plan.flows ?? []).filter((f) => {
    const seam = (map.seams ?? []).find((s) => s.id === f.seam);
    return seam !== undefined && !seamJoins(seam, f.from, f.to);
  });
}

/** Data models whose part is not a row of Affected parts (ADR 0031). */
export function modelsOffPart(plan: Plan): DataModel[] {
  const parts = new Set(plan.affected_parts.map((p) => p.node));
  return (plan.data_models ?? []).filter((m) => !parts.has(m.part));
}

/** Data models that cross a seam no row of Affected parts touches (ADR 0031). */
export function modelsOffSeam(plan: Plan): DataModel[] {
  const seams = new Set(plan.affected_parts.flatMap((p) => p.seams ?? []));
  return (plan.data_models ?? []).filter((m) => m.crosses !== undefined && !seams.has(m.crosses));
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
  for (const m of plan.data_models ?? []) {
    ids.add(m.part);
    if (m.crosses !== undefined) {
      ids.add(m.crosses);
    }
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
    dataModelsChecked: 0,
    modelsOffPart: [],
    modelsOffSeam: [],
    legacyKeys: legacyKeys(text),
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
  result.dataModelsChecked = plan.data_models?.length ?? 0;
  result.modelsOffPart = modelsOffPart(plan).map((m) => `${m.model} on ${m.part}`);
  result.modelsOffSeam = modelsOffSeam(plan).map((m) => `${m.model} over ${m.crosses}`);
  result.ok =
    result.schemaErrors.length === 0 &&
    result.unknownIds.length === 0 &&
    result.seamsWithoutORow.length === 0 &&
    result.flowsOffSeam.length === 0 &&
    result.modelsOffPart.length === 0 &&
    result.modelsOffSeam.length === 0;
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
    data_models: r.dataModelsChecked,
    models_off_part: r.modelsOffPart,
    models_off_seam: r.modelsOffSeam,
    legacy_keys: r.legacyKeys.length,
    legacy_key_names: r.legacyKeys,
    schema_errors: r.schemaErrors,
    parse_error: r.parseError,
    ok: r.ok,
  };
}
