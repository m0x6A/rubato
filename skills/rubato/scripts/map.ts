#!/usr/bin/env -S node --experimental-strip-types
/**
 * Print the system map for the session: every node and seam id the plan may use, the map's
 * git sha (`rubato.map.version`), and the plan path for the current repo. Validates the map
 * against the schema first; a map that fails is refused, not offered.
 *
 *   map.ts [--json] [--cwd <dir>]
 */
import { existsSync } from "node:fs";
import { planCheck, schemaDir } from "./lib/vendor.ts";
import { headSha, originRepo } from "./lib/git.ts";
import { loadWorkspace, mapFile, repoForCwd, type Workspace } from "./lib/workspace.ts";

export interface MapSummary {
  map_file: string;
  map_version: string;
  name: string;
  nodes: { id: string; kind: string; repo?: string; description?: string }[];
  seams: { id: string; from: string; to: string; owner: string; carries?: string }[];
  repo: string | null;
  plan_path_template: string;
}

export async function summarize(ws: Workspace, cwd: string): Promise<MapSummary> {
  const { map: mapMod } = await planCheck();
  const file = mapFile(ws);
  if (!existsSync(file)) {
    throw new Error(`map file ${file} not found; is ${ws.map.repo} cloned at ${ws.map.local}?`);
  }
  const map = mapMod.loadMap(file, schemaDir());
  const repo = repoForCwd(ws, cwd)?.name ?? originRepo(cwd);
  const override = repo ? map.repos?.find((r) => r.name === repo)?.plan_path : undefined;
  return {
    map_file: file,
    map_version: headSha(ws.map.local),
    name: map.name,
    nodes: map.nodes.map((n) => {
      const out: MapSummary["nodes"][number] = { id: n.id, kind: n.kind };
      if (n.repo) {
        out.repo = n.repo;
      }
      if (n.description) {
        out.description = n.description;
      }
      return out;
    }),
    seams: (map.seams ?? []).map((s) => {
      const out: MapSummary["seams"][number] = { id: s.id, from: s.from, to: s.to, owner: s.owner };
      if (s.carries) {
        out.carries = s.carries;
      }
      return out;
    }),
    repo,
    plan_path_template: override ?? "docs/plans/{issue}.md",
  };
}

export function render(s: MapSummary): string {
  const lines = [
    `# System map: ${s.name}`,
    `map: ${s.map_file} @ ${s.map_version} (rubato.map.version)`,
    `repo: ${s.repo ?? "(not a connected repo)"}; plan path: ${s.plan_path_template}`,
    "",
    "## Nodes (use these ids, invent none)",
    ...s.nodes.map((n) => `- ${n.id} [${n.kind}]${n.repo ? ` ${n.repo}` : ""}${n.description ? ` — ${n.description}` : ""}`),
    "",
    "## Seams (each one you touch needs an O-row)",
    ...s.seams.map((x) => `- ${x.id}: ${x.from} → ${x.to}, owner ${x.owner}${x.carries ? ` — ${x.carries}` : ""}`),
  ];
  return lines.join("\n");
}

async function main(argv: string[]): Promise<number> {
  const json = argv.includes("--json");
  const cwdIdx = argv.indexOf("--cwd");
  const cwd = cwdIdx >= 0 ? (argv[cwdIdx + 1] ?? process.cwd()) : process.cwd();
  try {
    const s = await summarize(loadWorkspace(), cwd);
    console.log(json ? JSON.stringify(s, null, 2) : render(s));
    return 0;
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = await main(process.argv.slice(2));
}
