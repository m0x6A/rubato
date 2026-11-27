/**
 * Which tenant and workspace this page is on (issue #89, ADR 0049; issue #345, ADR 0089), and
 * the paths that follow from them.
 *
 * The tenant is one of those `GET /api/me` lists: `?t=<id>`, else the last one chosen, else the
 * first listed. The workspace lives in `?w=<id>`, so a link names it, and the last one chosen in
 * local storage, so a new tab opens where the person left off. Every tenant call goes under
 * `/api/t/{tenant}`, and a map-scoped one under `/api/t/{tenant}/w/{ws}`. Person calls (auth,
 * pairing, `/api/me`) never take a prefix.
 *
 * Until the page has learned its tenant, paths keep the form they had before tenants, which a
 * server answers 404: the shell learns the tenant from `/api/me` before its first tenant call.
 */
import type { WebEvent } from "../protocol.ts";

const STORAGE_KEY = "rubato.workspace";
const TENANT_KEY = "rubato.tenant";
/** Learned from `/api/me`, for a page with no `?t=` and nothing stored. Not written anywhere. */
let learnedTenant: string | null = null;

function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** The tenant this page acts in: `?t=`, else the last choice, else the first `/api/me` listed. */
export function currentTenant(): string | null {
  return new URLSearchParams(location.search).get("t") ?? stored(TENANT_KEY) ?? learnedTenant;
}

/**
 * `/api/me` said which tenants the person is in. A tenant the page asked for (`?t=`, or stored)
 * that is not among them is dropped; with none chosen the first is taken. `false` when the
 * person is in no tenant. A list that is absent (a server from before tenants) changes nothing.
 */
export function learnTenants(ids: string[] | undefined): boolean {
  if (ids === undefined) {
    return true;
  }
  const asked = currentTenant();
  if (asked !== null && !ids.includes(asked)) {
    forgetTenant();
  }
  learnedTenant = currentTenant() ?? ids[0] ?? null;
  return learnedTenant !== null;
}

/** The person chose a tenant: it goes into the URL and is remembered; its workspace is chosen anew. */
export function setTenant(id: string): void {
  const url = new URL(location.href);
  url.searchParams.set("t", id);
  url.searchParams.delete("w");
  history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  try {
    localStorage.setItem(TENANT_KEY, id);
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // A private window: the URL still says it.
  }
  learnedTenant = id;
  learned = null;
}

/** The person is not in the chosen tenant (any more): drop it everywhere. */
export function forgetTenant(): void {
  const url = new URL(location.href);
  url.searchParams.delete("t");
  history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  try {
    localStorage.removeItem(TENANT_KEY);
  } catch {
    // nothing stored
  }
  learnedTenant = null;
}

/** A tenant route for the current tenant (ADR 0089): `/api/queue` becomes `/api/t/acme/queue`. */
export function inTenant(path: string): string {
  const tenant = currentTenant();
  if (!tenant || !path.startsWith("/api/")) {
    return path;
  }
  return `/api/t/${encodeURIComponent(tenant)}/${path.slice("/api/".length)}`;
}

/** The path of one workspace of the current tenant itself, for deleting it. */
export function workspacePath(ws: string): string {
  const tenant = currentTenant();
  return tenant ? `/api/t/${encodeURIComponent(tenant)}/w/${encodeURIComponent(ws)}` : `/api/w/${encodeURIComponent(ws)}`;
}
export const DEFAULT_WORKSPACE = "default";
/** Learned from an overview, for a page with no `?w=` and nothing stored. Not written anywhere. */
let learned: string | null = null;

function fromUrl(): string | null {
  return new URLSearchParams(location.search).get("w");
}

function fromStorage(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

/** The workspace map-scoped calls go to: `?w=`, else the last choice, else what an overview said. */
export function currentWorkspace(): string | null {
  return fromUrl() ?? fromStorage() ?? learned;
}

/** The overview named its workspace: scope later calls to it, without touching the URL. */
export function learnWorkspace(id: string): void {
  learned = id;
}

/** The person chose a workspace: it goes into the URL and is remembered. */
export function setWorkspace(id: string): void {
  const url = new URL(location.href);
  url.searchParams.set("w", id);
  history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    // A private window: the URL still says it.
  }
  learned = id;
}

/** The server does not hold the chosen workspace (any more): drop it everywhere. */
export function forgetWorkspace(): void {
  const url = new URL(location.href);
  url.searchParams.delete("w");
  history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // nothing stored
  }
  learned = null;
}

/**
 * A map-scoped API path for the current tenant and workspace (ADR 0089): `/api/pins` becomes
 * `/api/t/acme/w/shop/pins`. With no workspace learned yet it asks for `default`; a tenant
 * without one answers `unknown_workspace`, and the overview then takes the tenant's first.
 */
export function scoped(path: string): string {
  const ws = currentWorkspace();
  const tenant = currentTenant();
  if (tenant && path.startsWith("/api/")) {
    return `/api/t/${encodeURIComponent(tenant)}/w/${encodeURIComponent(ws ?? DEFAULT_WORKSPACE)}/${path.slice("/api/".length)}`;
  }
  // Unprefixed is `default` (ADR 0049), so `default` keeps its plain paths: one form for a
  // single-workspace team, and a newer page still works against an older server.
  if (!ws || ws === DEFAULT_WORKSPACE || !path.startsWith("/api/")) {
    return path;
  }
  return `/api/w/${encodeURIComponent(ws)}/${path.slice("/api/".length)}`;
}

/**
 * Whether a change means refetching the overview: one for every workspace, or for ours. An
 * `updated` event names one resource the overview does not carry, so only its view refetches.
 */
export function concernsUs(ev: WebEvent): boolean {
  if (ev.type !== "changed") {
    return false;
  }
  return !ev.workspace || ev.workspace === (currentWorkspace() ?? DEFAULT_WORKSPACE);
}

/** The server's answer for a tenant that does not exist or that the person is not in (ADR 0089). */
export function isUnknownTenant(status: number, body: string): boolean {
  return status === 404 && body.includes("unknown_tenant");
}

/** The server's answer for a workspace it does not hold (`kernel::http::UNKNOWN_WORKSPACE`). */
export function isUnknownWorkspace(status: number, body: string): boolean {
  return status === 404 && body.includes("unknown_workspace");
}
