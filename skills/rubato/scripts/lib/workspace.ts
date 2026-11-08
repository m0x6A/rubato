/**
 * The developer's `workspace.yml`: where the map repo is cloned, which repos are connected,
 * the server URL and the OTel endpoint. Looked up in `$RUBATO_WORKSPACE`, then
 * `$XDG_CONFIG_HOME/rubato/workspace.yml`, then `~/.config/rubato/workspace.yml`.
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

export interface Workspace {
  server_url?: string;
  map: {
    repo: string;
    path: string;
    local: string;
    /** Directory of ADRs inside the map repo. Default `docs/decisions`. */
    decisions?: string;
  };
  repos: WorkspaceRepo[];
  otel?: {
    endpoint: string;
    protocol?: "grpc" | "http/protobuf" | "http/json";
  };
}

export function workspaceCandidates(env: NodeJS.ProcessEnv = process.env): string[] {
  const out: string[] = [];
  if (env["RUBATO_WORKSPACE"]) {
    out.push(env["RUBATO_WORKSPACE"]);
  }
  const xdg = env["XDG_CONFIG_HOME"];
  if (xdg) {
    out.push(join(xdg, "rubato", "workspace.yml"));
  }
  out.push(join(homedir(), ".config", "rubato", "workspace.yml"));
  return out;
}

export class WorkspaceError extends Error {}

export function parseWorkspace(text: string, source: string): Workspace {
  const data = parseYaml(text) as Partial<Workspace> | null;
  if (!data || typeof data !== "object") {
    throw new WorkspaceError(`${source}: empty or not a mapping`);
  }
  const map = data.map;
  if (!map || typeof map.repo !== "string" || typeof map.path !== "string" || typeof map.local !== "string") {
    throw new WorkspaceError(`${source}: map needs repo, path and local`);
  }
  const repos = Array.isArray(data.repos) ? data.repos : [];
  for (const r of repos) {
    if (typeof r?.name !== "string" || typeof r?.local !== "string") {
      throw new WorkspaceError(`${source}: every repo needs name and local`);
    }
  }
  const ws: Workspace = { map: { repo: map.repo, path: map.path, local: resolve(map.local) }, repos: repos.map((r) => ({ name: r.name, local: resolve(r.local) })) };
  if (typeof map.decisions === "string") {
    ws.map.decisions = map.decisions;
  }
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

export function loadWorkspace(env: NodeJS.ProcessEnv = process.env): Workspace {
  for (const c of workspaceCandidates(env)) {
    if (existsSync(c)) {
      return parseWorkspace(readFileSync(c, "utf8"), c);
    }
  }
  throw new WorkspaceError(`no workspace.yml found; looked in ${workspaceCandidates(env).join(", ")}. See skills/rubato/workspace.example.yml`);
}

/** The connected repo whose clone contains `cwd`, if any. */
export function repoForCwd(ws: Workspace, cwd: string): WorkspaceRepo | undefined {
  const abs = resolve(cwd);
  return ws.repos.find((r) => abs === r.local || abs.startsWith(r.local + "/"));
}

export function mapFile(ws: Workspace): string {
  return join(ws.map.local, ws.map.path);
}
