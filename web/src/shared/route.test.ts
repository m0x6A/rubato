import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { HOME, type Route, currentRoute, formatRoute, keepRouteForLogin, navigate, parentOf, isPage, parseRoute, restoreRouteAfterLogin, sameView, showOnPage } from "./route.ts";

const ISSUE = { repo: "m0x6A/rubato", number: 357 };
const PART = { kind: "node", id: "rubato-server" } as const;
const SEAM = { kind: "seam", id: "web-server" } as const;

/** One route of every kind; the round trip below must cover them all. */
const EVERY: Route[] = [
  { view: "issues", list: "open", sel: null },
  { view: "issues", list: "open", sel: PART },
  { view: "issues", list: "not-pinned", sel: null },
  { view: "issues", list: "done", sel: SEAM },
  { view: "map", sel: PART },
  { view: "issue", issue: ISSUE, sel: null },
  { view: "issue", issue: ISSUE, sel: SEAM },
  { view: "new-issue", at: null, sel: null },
  { view: "new-issue", at: SEAM, sel: SEAM },
  { view: "plans", sel: null },
  { view: "plan", issue: ISSUE, sel: null },
  { view: "companions", sel: null },
  { view: "repos", sel: PART },
  { view: "findings", sel: null },
  { view: "settings", sel: null },
  { view: "workspaces", sel: null },
  { view: "members", sel: null },
  { view: "operator", sel: null },
  { view: "not-found", fragment: "/nowhere", sel: null },
];

const KINDS = ["map", "issues", "issue", "new-issue", "plans", "plan", "companions", "repos", "findings", "settings", "workspaces", "members", "operator", "not-found"];

/** How many of the routes survive `format` then `parse`, and which kinds they cover. */
function roundTrips(routes: Route[]): { ok: number; kinds: Set<string> } {
  let ok = 0;
  const kinds = new Set<string>();
  for (const r of routes) {
    if (JSON.stringify(parseRoute(formatRoute(r))) === JSON.stringify(r)) {
      ok += 1;
      kinds.add(r.view);
    }
  }
  return { ok, kinds };
}

describe("the view in the URL fragment (issue #357)", () => {
  beforeEach(() => {
    history.replaceState(null, "", "/");
    sessionStorage.clear();
  });
  afterEach(() => history.replaceState(null, "", "/"));

  it("every kind of route survives format then parse", () => {
    const { ok, kinds } = roundTrips(EVERY);
    console.log(`route round trip: ${ok} of ${EVERY.length} routes, ${kinds.size} of ${KINDS.length} kinds`);
    expect(ok).toBe(EVERY.length);
    expect([...kinds].sort()).toEqual([...KINDS].sort());
  });

  it("must fail: the round trip notices a kind that is not covered", () => {
    const { kinds } = roundTrips(EVERY.filter((r) => r.view !== "plan"));
    expect([...kinds].sort()).not.toEqual([...KINDS].sort());
  });

  it("reads the fragments a person would paste", () => {
    expect(parseRoute("")).toEqual(HOME);
    expect(parseRoute("#/")).toEqual(HOME);
    expect(parseRoute("#/issues/m0x6A/rubato/357")).toEqual({ view: "issue", issue: ISSUE, sel: null });
    expect(parseRoute("#/issues?sel=part:rubato-server")).toEqual({ view: "issues", list: "open", sel: PART });
    expect(parseRoute("#/issues/not-pinned")).toEqual({ view: "issues", list: "not-pinned", sel: null });
    expect(parseRoute("#/map?sel=seam:web-server")).toEqual({ view: "map", sel: SEAM });
    expect(parseRoute("#/issues/new?at=seam:web-server")).toEqual({ view: "new-issue", at: SEAM, sel: null });
    expect(formatRoute({ view: "issue", issue: ISSUE, sel: SEAM })).toBe("#/issues/m0x6A/rubato/357?sel=seam:web-server");
  });

  it("names nothing it cannot read", () => {
    for (const hash of ["#/nowhere", "#issues", "#/issues/m0x6A/rubato", "#/issues/m0x6A/rubato/0", "#/issues/m0x6A/rubato/12/extra/more", "#/plans/x", "#/repos/extra", "#/map", "#/issues/done/more", "#/settings/nope", "#/issues/new?at=galaxy:x", "#/issues/%E0%A4%A"]) {
      expect(parseRoute(hash).view, hash).toBe("not-found");
    }
    // A selection that reads as nothing is dropped; the view still shows.
    expect(parseRoute("#/issues?sel=galaxy:x")).toEqual(HOME);
  });

  it("the way up closes the view first and the selection last", () => {
    // An issue opened beside a selected part closes to that part's inspector, which closes to the list.
    expect(parentOf({ view: "issue", issue: ISSUE, sel: PART })).toEqual({ view: "map", sel: PART });
    expect(parentOf({ view: "map", sel: PART })).toEqual(HOME);
    expect(parentOf({ view: "issue", issue: ISSUE, sel: null })).toEqual(HOME);
    expect(parentOf({ view: "issues", list: "open", sel: PART })).toEqual(HOME);
    expect(parentOf({ view: "issues", list: "done", sel: null })).toEqual(HOME);
    expect(parentOf(HOME)).toBeNull();
    expect(parentOf({ view: "plan", issue: ISSUE, sel: null })).toEqual({ view: "plans", sel: null });
    expect(parentOf({ view: "new-issue", at: PART, sel: PART })).toEqual({ view: "map", sel: PART });
    expect(parentOf({ view: "companions", sel: null })).toBeNull();
    expect(parentOf({ view: "settings", sel: null })).toEqual(HOME);
    expect(parentOf({ view: "findings", sel: PART })).toEqual({ view: "issues", list: "open", sel: PART });
    expect(parseRoute("#/settings/workspaces")).toEqual({ view: "workspaces", sel: null });
    expect(isPage({ view: "workspaces", sel: null })).toBe(true);
    expect(isPage({ view: "findings", sel: null })).toBe(false);
  });

  it("a new view is a new history entry; another selection on the same view is not", () => {
    const before = history.length;
    navigate({ view: "plans", sel: null });
    expect(location.hash).toBe("#/plans");
    expect(history.length).toBe(before + 1);
    navigate({ view: "plans", sel: PART });
    expect(location.hash).toBe("#/plans?sel=part:rubato-server");
    expect(history.length).toBe(before + 1);
    navigate({ view: "companions", sel: PART }, { replace: true });
    expect(history.length).toBe(before + 1);
    navigate({ view: "companions", sel: PART });
    expect(history.length).toBe(before + 1);
    expect(currentRoute()).toEqual({ view: "companions", sel: PART });
    expect(sameView({ view: "issue", issue: ISSUE, sel: PART }, { view: "issue", issue: ISSUE, sel: null })).toBe(true);
  });

  it("navigating keeps the tenant and the workspace in the query", () => {
    history.replaceState(null, "", "/?t=acme&w=shop");
    navigate({ view: "repos", sel: null });
    expect(location.search).toBe("?t=acme&w=shop");
    expect(location.hash).toBe("#/repos");
  });

  it("the view asked for before the login is shown after it, once", () => {
    history.replaceState(null, "", "/#/issues/m0x6A/rubato/357");
    keepRouteForLogin();
    // The login returns to the page with no fragment.
    history.replaceState(null, "", "/");
    restoreRouteAfterLogin();
    expect(currentRoute()).toEqual({ view: "issue", issue: ISSUE, sel: null });
    history.replaceState(null, "", "/");
    restoreRouteAfterLogin();
    expect(currentRoute()).toEqual(HOME);
  });

  it("the tenant and the workspace of the address are kept across the login too", () => {
    history.replaceState(null, "", "/?t=acme&w=shop#/issues/m0x6A/rubato/357");
    keepRouteForLogin();
    history.replaceState(null, "", "/");
    restoreRouteAfterLogin();
    expect(location.search).toBe("?t=acme&w=shop");
    expect(currentRoute()).toEqual({ view: "issue", issue: ISSUE, sel: null });
  });

  it("a link to something on the page focuses it and leaves the fragment, which names the view", () => {
    history.replaceState(null, "", "/#/issues/m0x6A/rubato/357");
    const target = document.createElement("li");
    target.id = "log-entry-7";
    target.scrollIntoView = () => {};
    document.body.append(target);
    let prevented = false;
    showOnPage({ preventDefault: () => (prevented = true) }, "log-entry-7");
    expect(prevented).toBe(true);
    expect(document.activeElement).toBe(target);
    expect(location.hash).toBe("#/issues/m0x6A/rubato/357");
    // Nothing by that id: still no change of view.
    showOnPage({ preventDefault: () => {} }, "gone");
    expect(location.hash).toBe("#/issues/m0x6A/rubato/357");
    target.remove();
  });

  it("a fragment the person came back with wins over the one kept", () => {
    history.replaceState(null, "", "/#/plans");
    keepRouteForLogin();
    history.replaceState(null, "", "/#/repos");
    restoreRouteAfterLogin();
    expect(location.hash).toBe("#/repos");
  });
});
