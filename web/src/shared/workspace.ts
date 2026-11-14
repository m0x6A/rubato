/**
 * Which workspace this page is on (issue #89, ADR 0049), and the paths that follow from it.
 *
 * The choice lives in `?w=<id>`, so a link names it, and the last one chosen in local storage,
 * so a new tab opens where the person left off. Before the first overview says which workspace
 * it is, and always for `default`, map-scoped calls go unprefixed, which the server answers as
 * `default`. Team calls
 * (queue, companions, auth, `/ws/web`) never take a prefix: the server mounts them once.
 */
import type { WebEvent } from "../protocol.ts";

const STORAGE_KEY = "rubato.workspace";
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

/** A map-scoped API path for the current workspace: `/api/pins` becomes `/api/w/shop/pins`. */
export function scoped(path: string): string {
  const ws = currentWorkspace();
  // Unprefixed is `default` (ADR 0049), so `default` keeps its plain paths: one form for a
  // single-workspace team, and a newer page still works against an older server.
  if (!ws || ws === DEFAULT_WORKSPACE || !path.startsWith("/api/")) {
    return path;
  }
  return `/api/w/${encodeURIComponent(ws)}/${path.slice("/api/".length)}`;
}

/** Whether a change concerns this page: one for every workspace, or for ours. */
export function concernsUs(ev: WebEvent): boolean {
  return !ev.workspace || ev.workspace === (currentWorkspace() ?? DEFAULT_WORKSPACE);
}

/** The server's answer for a workspace it does not hold (`kernel::http::UNKNOWN_WORKSPACE`). */
export function isUnknownWorkspace(status: number, body: string): boolean {
  return status === 404 && body.includes("unknown_workspace");
}
