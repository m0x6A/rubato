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
  for (const id of [...map.nodes.map((n) => n.id), ...(map.seams ?? []).map((s) => s.id)]) {
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
