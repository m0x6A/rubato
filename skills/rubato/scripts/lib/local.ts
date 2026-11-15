/**
 * The developer's local config (ADR 0044): where each map repo is cloned, which repos are
 * connected, the server URL and the OTel endpoint. Looked up in `$RUBATO_LOCAL`, then
 * `$XDG_CONFIG_HOME/rubato/local.yml`, then `~/.config/rubato/local.yml`; the old names
 * (`$RUBATO_WORKSPACE`, `workspace.yml`) are still read, with one deprecation warning.
 *
 * Several maps may be listed under `maps:`, one per workspace whose map the developer plans
 * against; a single `map:` is read as a list of one.
 */
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { parse as parseYaml } from "yaml";

export interface WorkspaceRepo {
  /** `owner/name`. */
  name: string;
  /** Local clone. */
  local: string;
}

/** One map repo's local clone. */
export interface MapClone {
  repo: string;
  path: string;
  local: string;
  /** Directory of ADRs inside the map repo. Default `docs/decisions`. */
  decisions?: string;
}

export interface Workspace {
  server_url?: string;
  /** Every map clone, in the order listed. */
  maps: MapClone[];
  /** The first of `maps`: what a single-map config named `map`. */
  map: MapClone;
  repos: WorkspaceRepo[];
  otel?: {
    endpoint: string;
    protocol?: "grpc" | "http/protobuf" | "http/json";
  };
}

/** The local config, by its new name. */
export type LocalConfig = Workspace;

/** Where the config is looked for: the new names first, then the old ones. */
export function workspaceCandidates(env: NodeJS.ProcessEnv = process.env): string[] {
  const out: string[] = [];
  const xdg = env["XDG_CONFIG_HOME"];
  for (const [envName, file] of [
    ["RUBATO_LOCAL", "local.yml"],
    ["RUBATO_WORKSPACE", "workspace.yml"],
  ] as const) {
    if (env[envName]) {
      out.push(env[envName] as string);
    }
    if (xdg) {
      out.push(join(xdg, "rubato", file));
    }
    out.push(join(homedir(), ".config", "rubato", file));
  }
  return out;
}

export class WorkspaceError extends Error {}

function clone(m: Partial<MapClone> | undefined, source: string): MapClone {
  if (!m || typeof m.repo !== "string" || typeof m.path !== "string" || typeof m.local !== "string") {
    throw new WorkspaceError(`${source}: map needs repo, path and local`);
  }
  const out: MapClone = { repo: m.repo, path: m.path, local: resolve(m.local) };
  if (typeof m.decisions === "string") {
    out.decisions = m.decisions;
  }
  return out;
}

export function parseWorkspace(text: string, source: string): Workspace {
  const data = parseYaml(text) as (Partial<Omit<Workspace, "maps" | "map">> & { map?: Partial<MapClone>; maps?: Partial<MapClone>[] }) | null;
  if (!data || typeof data !== "object") {
    throw new WorkspaceError(`${source}: empty or not a mapping`);
  }
  const listed = Array.isArray(data.maps) ? data.maps : data.map ? [data.map] : [];
  if (listed.length === 0) {
    throw new WorkspaceError(`${source}: map needs repo, path and local (or list them under maps:)`);
  }
  const maps = listed.map((m) => clone(m, source));
  const repos = Array.isArray(data.repos) ? data.repos : [];
  for (const r of repos) {
    if (typeof r?.name !== "string" || typeof r?.local !== "string") {
      throw new WorkspaceError(`${source}: every repo needs name and local`);
    }
  }
  const ws: Workspace = { maps, map: maps[0] as MapClone, repos: repos.map((r) => ({ name: r.name, local: resolve(r.local) })) };
  if (typeof data.server_url === "string") {
    ws.server_url = data.server_url;
  }
  if (data.otel && typeof data.otel.endpoint === "string") {
    ws.otel = { endpoint: data.otel.endpoint };
    if (data.otel.protocol) {
      ws.otel.protocol = data.otel.protocol;
    }
  }
  return ws;
}

let warned = false;

export function loadWorkspace(env: NodeJS.ProcessEnv = process.env, warn: (s: string) => void = (s) => console.error(s)): Workspace {
  for (const c of workspaceCandidates(env)) {
    if (existsSync(c)) {
      if (!warned && (c.endsWith("workspace.yml") || c === env["RUBATO_WORKSPACE"])) {
        warned = true;
        warn(`${c}: workspace.yml is now local.yml (and RUBATO_WORKSPACE is RUBATO_LOCAL); rename it, after upgrading the companion`);
      }
      return parseWorkspace(readFileSync(c, "utf8"), c);
    }
  }
  throw new WorkspaceError(`no local.yml found; looked in ${workspaceCandidates(env).join(", ")}. See skills/rubato/local.example.yml`);
}

export const loadLocal = loadWorkspace;
export const parseLocal = parseWorkspace;

/** The connected repo whose clone contains `cwd`, if any. */
export function repoForCwd(ws: Workspace, cwd: string): WorkspaceRepo | undefined {
  const abs = resolve(cwd);
  return ws.repos.find((r) => abs === r.local || abs.startsWith(r.local + "/"));
}

/** The map file of a clone; without one, of the first map. */
export function mapFile(ws: Workspace, m: MapClone = ws.map): string {
  return join(m.local, m.path);
}

/** What a map names, enough to tell whether it holds a repo. */
export interface NamesRepos {
  repos?: { name: string }[] | undefined;
  nodes: { repo?: string | null | undefined }[];
}

export function namesRepo(map: NamesRepos, repo: string): boolean {
  const r = repo.toLowerCase();
  return (map.repos ?? []).some((x) => x.name.toLowerCase() === r) || map.nodes.some((n) => (n.repo ?? "").toLowerCase() === r);
}

/**
 * The maps a session in `repo` plans against (ADR 0045): every listed map that names the repo.
 * None names it: with one map, that map (as before workspaces); with several, none.
 */
export function matchedMaps<M extends NamesRepos>(ws: Workspace, repo: string | null, load: (file: string) => M): { clone: MapClone; file: string; map: M }[] {
  const all = ws.maps.map((c) => ({ clone: c, file: mapFile(ws, c), map: load(mapFile(ws, c)) }));
  if (all.length === 1) {
    return all;
  }
  return repo ? all.filter((m) => namesRepo(m.map, repo)) : [];
}

/** The `rubato.map.select` line: which maps a session in `repo` got. */
export function selectLine(repo: string | null, matched: number, listed: number): string {
  const outcome = matched === 0 ? "none" : matched === 1 ? "one" : "several";
  return JSON.stringify({ event: "rubato.map.select", repo, maps_listed: listed, maps_matched: matched, outcome });
}
