/**
 * Load `system-map.yml`, validate it against `schema/system-map.schema.json`, and run the
 * structural checks the Rust side runs in `SystemMap::validate` (ADR 0007).
 */
import { readFileSync } from "node:fs";
import { parse as parseYaml } from "yaml";
import { validator } from "./schema.ts";

export interface MapNode {
  id: string;
  kind: "context" | "contract" | "external";
  repo?: string;
  path?: string;
  description?: string;
}

export interface Seam {
  id: string;
  from: string;
  to: string;
  carries?: string;
  owner: string;
  guard?: string;
  description?: string;
  /** What crosses the seam, one entry per thing and direction (ADR 0017). */
  flows?: Flow[];
}

export interface Flow {
  kind: "data" | "control";
  from: string;
  to: string;
  carries: string;
}

/** Whether a flow from `from` to `to` runs along the seam: its two nodes, either way. */
export function seamJoins(s: Seam, from: string, to: string): boolean {
  return (from === s.from && to === s.to) || (from === s.to && to === s.from);
}

/** A named journey: ordered steps over declared flows (ADR 0024). */
export interface Scenario {
  id: string;
  name: string;
  description?: string;
  steps: ScenarioStep[];
}

/** One declared flow on `seam`, picked by kind and direction, and `carries` when two match. */
export interface ScenarioStep {
  seam: string;
  from: string;
  to: string;
  kind: "data" | "control";
  carries?: string;
  says?: string;
}

/** The declared flows a step could mean. Same rule as `Seam::flows_matching` in Rust. */
export function flowsMatching(s: Seam, step: ScenarioStep): Flow[] {
  return (s.flows ?? []).filter(
    (f) => f.kind === step.kind && f.from === step.from && f.to === step.to && (step.carries === undefined || step.carries === f.carries),
  );
}

export interface RepoRef {
  name: string;
  plan_path?: string;
}

export interface SystemMap {
  version: number;
  name: string;
  owner_rule?: string;
  repos?: RepoRef[];
  nodes: MapNode[];
  seams?: Seam[];
  scenarios?: Scenario[];
}

export class MapError extends Error {
  readonly problems: string[];

  constructor(message: string, problems: string[]) {
    super(message);
    this.name = "MapError";
    this.problems = problems;
  }
}

export function structuralProblems(map: SystemMap): string[] {
  const problems: string[] = [];
  const seen = new Set<string>();
  for (const id of [...map.nodes.map((n) => n.id), ...(map.seams ?? []).map((s) => s.id), ...(map.scenarios ?? []).map((s) => s.id)]) {
    if (seen.has(id)) {
      problems.push(`duplicate id ${JSON.stringify(id)}`);
    }
    seen.add(id);
  }
  const nodeIds = new Set(map.nodes.map((n) => n.id));
  for (const s of map.seams ?? []) {
    for (const node of [s.from, s.to]) {
      if (!nodeIds.has(node)) {
        problems.push(`seam ${JSON.stringify(s.id)} references unknown node ${JSON.stringify(node)}`);
      }
    }
    if (s.owner !== s.from && s.owner !== s.to) {
      problems.push(`seam ${JSON.stringify(s.id)} owner ${JSON.stringify(s.owner)} is neither its from nor its to side`);
    }
    for (const f of s.flows ?? []) {
      if (!seamJoins(s, f.from, f.to)) {
        problems.push(`seam ${JSON.stringify(s.id)} has a flow from ${JSON.stringify(f.from)} to ${JSON.stringify(f.to)}, which are not its two nodes`);
      }
    }
  }
  const q = JSON.stringify;
  for (const sc of map.scenarios ?? []) {
    if (sc.steps.length === 0) {
      problems.push(`scenario ${q(sc.id)} has no steps`);
    }
    sc.steps.forEach((step, i) => {
      const seam = (map.seams ?? []).find((s) => s.id === step.seam);
      if (!seam) {
        problems.push(`scenario ${q(sc.id)} step ${i + 1} names unknown seam ${q(step.seam)}`);
        return;
      }
      const n = flowsMatching(seam, step).length;
      const at = `scenario ${q(sc.id)} step ${i + 1} on seam ${q(step.seam)}`;
      if (n === 0) {
        const carrying = step.carries === undefined ? "" : ` carrying ${q(step.carries)}`;
        problems.push(`${at}: no declared ${step.kind} flow from ${q(step.from)} to ${q(step.to)}${carrying}`);
      } else if (n > 1) {
        problems.push(`${at}: ${n} declared ${step.kind} flows from ${q(step.from)} to ${q(step.to)}; name one with carries`);
      }
    });
  }
  return problems;
}

/** Parse and validate map text. Throws `MapError` listing every problem. */
export function loadMapText(text: string, schemaDir: string): SystemMap {
  const data = parseYaml(text) as unknown;
  const validate = validator(schemaDir, "system-map.schema.json");
  if (!validate(data)) {
    const problems = (validate.errors ?? []).map((e) => `${e.instancePath || "/"} ${e.message ?? ""}`.trim());
    throw new MapError("system map fails schema/system-map.schema.json", problems);
  }
  const map = data as SystemMap;
  const problems = structuralProblems(map);
  if (problems.length) {
    throw new MapError("system map has structural problems", problems);
  }
  return map;
}

export function loadMap(path: string, schemaDir: string): SystemMap {
  return loadMapText(readFileSync(path, "utf8"), schemaDir);
}

/** Every id a plan may reference. */
export function mapIds(map: SystemMap): Set<string> {
  return new Set([...map.nodes.map((n) => n.id), ...(map.seams ?? []).map((s) => s.id)]);
}
