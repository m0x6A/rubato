/**
 * Which view the page shows, read from the URL fragment (issue #357).
 *
 * The fragment names the view (`#/issues/m0x6A/rubato/357`), and the map selection rides inside
 * it as `?sel=part:<id>`, because a part can stay selected while an issue is open. The tenant and
 * the workspace stay in the query (`shared/workspace.ts`). The fragment, not the path: the server
 * answers an unknown path with the page and status 404.
 *
 * A change of view is a new history entry; a change of only the selection replaces the entry.
 * The way up from a view (`parentOf`) is where Escape and a back link go: never `history.back()`,
 * since a pasted link has no history.
 */
import { useSyncExternalStore } from "react";
import type { IssueRef, MapRef } from "../protocol.ts";

/** Which issues the list shows: open ones Rubato tracks, open ones nobody pinned, or the finished. */
export type IssueList = "open" | "not-pinned" | "done";

/** The parts of an issue's view, one at a time; the overview is the issue's own address. */
export type IssueSection = "overview" | "plan" | "room" | "sessions" | "pr";

export type View =
  | { view: "issues"; list: IssueList }
  /** The inspector of the part or seam selected on the map. */
  | { view: "map" }
  | { view: "issue"; issue: IssueRef; section: IssueSection }
  | { view: "new-issue"; at: MapRef | null }
  | { view: "plans" }
  | { view: "plan"; issue: IssueRef }
  | { view: "companions" }
  | { view: "repos" }
  | { view: "findings" }
  | { view: "settings" }
  | { view: "workspaces" }
  | { view: "members" }
  | { view: "operator" }
  | { view: "not-found"; fragment: string };

/** A view, and the part or seam selected on the map beside it. */
export type Route = View & { sel: MapRef | null };

export const HOME: Route = { view: "issues", list: "open", sel: null };

const ID = /^[A-Za-z0-9._-]+$/;

/** The lists of the issue view that have a path of their own; the open list is `/issues`. */
const LISTS: Record<string, IssueList> = { "not-pinned": "not-pinned", done: "done" };

/** The sections of an issue's view that have a path of their own. */
const SECTIONS: Record<string, IssueSection> = { plan: "plan", room: "room", sessions: "sessions", pr: "pr" };

function parseRef(s: string | null): MapRef | null {
  if (!s) {
    return null;
  }
  const colon = s.indexOf(":");
  const kind = s.slice(0, colon);
  const id = s.slice(colon + 1);
  if ((kind !== "part" && kind !== "seam") || !ID.test(id)) {
    return null;
  }
  return { kind: kind === "part" ? "node" : "seam", id };
}

function formatRef(ref: MapRef): string {
  return `${ref.kind === "seam" ? "seam" : "part"}:${ref.id}`;
}

function parseIssue(owner: string | undefined, repo: string | undefined, n: string | undefined): IssueRef | null {
  if (!owner || !repo || !n || !ID.test(owner) || !ID.test(repo) || !/^[1-9][0-9]*$/.test(n)) {
    return null;
  }
  return { repo: `${owner}/${repo}`, number: Number(n) };
}

/** The route a fragment names. An empty one is the issue list; one that names nothing is `not-found`. */
export function parseRoute(hash: string): Route {
  const fragment = hash.startsWith("#") ? hash.slice(1) : hash;
  if (fragment === "" || fragment === "/") {
    return HOME;
  }
  const q = fragment.indexOf("?");
  const path = q < 0 ? fragment : fragment.slice(0, q);
  const params = new URLSearchParams(q < 0 ? "" : fragment.slice(q + 1));
  const sel = parseRef(params.get("sel"));
  const notFound: Route = { view: "not-found", fragment, sel: null };
  if (!path.startsWith("/")) {
    return notFound;
  }
  let parts: string[];
  try {
    parts = path.slice(1).split("/").map(decodeURIComponent);
  } catch {
    return notFound;
  }
  const [head, a, b, c, d, ...rest] = parts;
  if (rest.length > 0) {
    return notFound;
  }
  const only = a === undefined;
  switch (head) {
    case "issues": {
      if (only) {
        return { view: "issues", list: "open", sel };
      }
      const named = a === undefined ? undefined : LISTS[a];
      if (named && b === undefined) {
        return { view: "issues", list: named, sel };
      }
      if (a === "new" && b === undefined) {
        const at = params.get("at");
        const ref = parseRef(at);
        return at !== null && ref === null ? notFound : { view: "new-issue", at: ref, sel };
      }
      const issue = parseIssue(a, b, c);
      const section = d === undefined ? "overview" : SECTIONS[d];
      return issue && section ? { view: "issue", issue, section, sel } : notFound;
    }
    case "plans": {
      if (only) {
        return { view: "plans", sel };
      }
      const issue = parseIssue(a, b, c);
      return issue && d === undefined ? { view: "plan", issue, sel } : notFound;
    }
    case "settings":
      if (d !== undefined || c !== undefined) {
        return notFound;
      }
      if (a === "workspaces" && b === undefined) {
        return { view: "workspaces", sel };
      }
      return only ? { view: "settings", sel } : notFound;
    case "map":
      // The inspector is of something: with nothing selected there is nothing to show.
      return only && sel ? { view: "map", sel } : notFound;
    case "companions":
    case "repos":
    case "findings":
    case "members":
    case "operator":
      return only ? { view: head, sel } : notFound;
    default:
      return notFound;
  }
}

/** The fragment of a route, `#` included. `parseRoute(formatRoute(r))` gives `r` back. */
export function formatRoute(route: Route): string {
  const params = new URLSearchParams();
  let path: string;
  switch (route.view) {
    case "issues":
      path = route.list === "open" ? "/issues" : `/issues/${route.list}`;
      break;
    case "issue":
      path = `/issues/${route.issue.repo}/${route.issue.number}${route.section === "overview" ? "" : `/${route.section}`}`;
      break;
    case "plan":
      path = `/plans/${route.issue.repo}/${route.issue.number}`;
      break;
    case "new-issue":
      path = "/issues/new";
      if (route.at) {
        params.set("at", formatRef(route.at));
      }
      break;
    case "workspaces":
      path = "/settings/workspaces";
      break;
    case "not-found":
      return `#${route.fragment}`;
    default:
      path = `/${route.view}`;
  }
  if (route.sel) {
    params.set("sel", formatRef(route.sel));
  }
  // The ids are plain (`ID`), so the colon reads better unescaped.
  const query = params.toString().replaceAll("%3A", ":");
  return `#${path}${query ? `?${query}` : ""}`;
}

/** The views that are a page of their own, with the map hidden: the settings, the people, the operator's. */
export function isPage(route: Route): boolean {
  return route.view === "settings" || route.view === "workspaces" || route.view === "members" || route.view === "operator";
}

/**
 * Whether two routes show the same view, whatever is selected on the map. An issue is one view
 * whichever of its sections shows: moving between them is not a step for Back.
 */
export function sameView(a: Route, b: Route): boolean {
  const flat = (r: Route): Route => (r.view === "issue" ? { ...r, section: "overview", sel: null } : { ...r, sel: null });
  return formatRoute(flat(a)) === formatRoute(flat(b));
}

/**
 * The way up from a route, or null at the top: where Escape and a back link lead. A selection is
 * the last thing to go, so an issue closes before the part it was opened from is deselected.
 */
export function parentOf(route: Route): Route | null {
  switch (route.view) {
    case "issue":
    case "new-issue":
      // Closed, the part it was opened beside shows again: its inspector.
      return route.sel ? { view: "map", sel: route.sel } : HOME;
    case "findings":
    case "not-found":
      return { view: "issues", list: "open", sel: route.sel };
    case "map":
      return HOME;
    case "issues":
      if (route.sel) {
        return { ...route, sel: null };
      }
      return route.list === "open" ? null : HOME;
    case "plan":
      return { view: "plans", sel: route.sel };
    case "settings":
    case "workspaces":
    case "members":
    case "operator":
      return { view: "issues", list: "open", sel: route.sel };
    default:
      return route.sel ? { ...route, sel: null } : null;
  }
}

const listeners = new Set<() => void>();

function emit(): void {
  for (const l of listeners) {
    l();
  }
}

function subscribe(listener: () => void): () => void {
  if (listeners.size === 0) {
    window.addEventListener("hashchange", emit);
    window.addEventListener("popstate", emit);
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      window.removeEventListener("hashchange", emit);
      window.removeEventListener("popstate", emit);
    }
  };
}

/** The route the URL names now. */
export function currentRoute(): Route {
  return parseRoute(location.hash);
}

/**
 * Show another route. A new view is a new history entry; the same view with another selection,
 * or `replace`, takes the place of the current one. Going where the page already is does nothing.
 */
export function navigate(route: Route, opts: { replace?: boolean } = {}): void {
  const hash = formatRoute(route);
  const here = currentRoute();
  if (formatRoute(here) === hash) {
    return;
  }
  const url = location.pathname + location.search + hash;
  if (opts.replace || sameView(here, route)) {
    history.replaceState(history.state, "", url);
  } else {
    history.pushState(null, "", url);
  }
  emit();
}

let cached: { hash: string; route: Route } | null = null;

function snapshot(): Route {
  const hash = location.hash;
  if (cached?.hash !== hash) {
    cached = { hash, route: parseRoute(hash) };
  }
  return cached.route;
}

/**
 * A link to something on the page itself (`<a href="#id">`): scroll to it and focus it, without
 * touching the fragment, which names the view.
 */
export function showOnPage(e: { preventDefault(): void }, id: string): void {
  e.preventDefault();
  const el = document.getElementById(id);
  if (!el) {
    return;
  }
  el.scrollIntoView({ block: "center" });
  if (el instanceof HTMLElement) {
    if (!el.hasAttribute("tabindex")) {
      el.setAttribute("tabindex", "-1");
    }
    el.focus({ preventScroll: true });
  }
}

/** The current route, kept in step with the URL: Back, Forward, a pasted link and `navigate`. */
export function useRoute(): Route {
  return useSyncExternalStore(subscribe, snapshot);
}

const AFTER_LOGIN = "rubato.after-login";

/**
 * The login page is about to send the person to GitHub: remember the address they asked for,
 * with its tenant and workspace, since an issue is an issue of one workspace.
 */
export function keepRouteForLogin(): void {
  try {
    if (location.hash.length > 1) {
      sessionStorage.setItem(AFTER_LOGIN, location.search + location.hash);
    }
  } catch {
    // No storage: the person lands on the issue list after logging in.
  }
}

/**
 * Back from the login, which returns to `/` with no fragment: show the view asked for before it.
 * Called once, before the first render.
 */
export function restoreRouteAfterLogin(): void {
  try {
    const kept = sessionStorage.getItem(AFTER_LOGIN);
    if (kept === null) {
      return;
    }
    sessionStorage.removeItem(AFTER_LOGIN);
    if (location.hash.length <= 1 && location.search === "") {
      history.replaceState(history.state, "", location.pathname + kept);
    }
  } catch {
    // nothing kept
  }
}
