import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AccountMenu, LoginPage, authApi } from "./features/auth/index.ts";
import { CompanionsPanel } from "./features/companions/index.ts";
import { DraftPanel, type DraftPrefill, draftMarks, draftsApi, seamOpFor, withDraft } from "./features/drafts/index.ts";
import { Findings } from "./features/findings/index.ts";
import { IssueList, IssuePanel, TeamTrack, ownConnectedCompanions } from "./features/issues/index.ts";
import { launchApi, launchFailureText, launchNotice } from "./features/launch/index.ts";
import { liveApi, subscribe, takeForgottenWorkspace } from "./features/live/index.ts";
import { NewIssueForm } from "./features/new_issue/index.ts";
import { Discrepancies, Inspector, MapView, PlanInspector, type Take, buildGraph, mapApi } from "./features/map/index.ts";
import { pinsApi } from "./features/pins/index.ts";
import { PlansBoard, groupPlans } from "./features/plans/index.ts";
import { ConnectedRepos, RepoView } from "./features/repos/index.ts";
import { PullView } from "./features/pulls/index.ts";
import { queueApi } from "./features/queue/index.ts";
import { SettingsView, WorkspacesView } from "./features/settings/index.ts";
import { WorkspaceMenu } from "./features/workspaces/index.ts";
import { MembersView } from "./features/tenants/index.ts";
import { AdminView } from "./features/admin/index.ts";
import type { DiscrepancyKey, IssueRef, MapLayout, MapRef, Me, Overview, PinRequest } from "./protocol.ts";
import { ApiError, failureText, onNeedsLogin, resetNeedsLogin } from "./shared/api.ts";
import { hasIssueDrag } from "./shared/dnd.ts";
import { openIssues } from "./shared/finished.ts";
import { issueKey, relative } from "./shared/format.ts";
import { concernsUs, currentTenant, forgetTenant, learnTenants, setTenant, setWorkspace } from "./shared/workspace.ts";
import { liveMap } from "./shared/map.ts";
import { PANEL_ACTION, type PanelSize, nextPanelSize, readPanelSize, storePanelSize, useSideBySide } from "./shared/panel.ts";
import { HOME, type Route, type View, currentRoute, isPage, navigate, parentOf, useRoute } from "./shared/route.ts";

type Tab = "issues" | "plans" | "companions" | "repos";

/**
 * The nav entry a route's view sits under, if any. An issue, a new issue and a link that names
 * nothing sit under Issues; the inspector and the findings are about the map, under no entry.
 */
function tabOf(route: Route): Tab | null {
  switch (route.view) {
    case "plans":
      return "plans";
    case "companions":
      return "companions";
    case "repos":
    case "repo":
    case "pull":
      return "repos";
    case "issue":
      // A plan read from the board is still under Plans: that is where its way back leads.
      return route.from ? "plans" : "issues";
    case "issues":
    case "new-issue":
    case "not-found":
      return "issues";
    default:
      return null;
  }
}

/** Show another view; what is selected on the map stays selected. */
function go(view: View, opts?: { replace?: boolean }): void {
  navigate({ ...view, sel: currentRoute().sel }, opts);
}

/** Close the view: up one level, as Escape does. */
function goUp(): void {
  const up = parentOf(currentRoute());
  if (up) {
    navigate(up);
  }
}

/** Select a part or a seam on the map, or nothing, without leaving the view. */
function selectOnly(sel: MapRef | null): void {
  navigate({ ...currentRoute(), sel });
}


/** A tab's count, quiet next to its name; the button's aria-label says what it counts. */
function TabCount({ n, live = false }: { n: number; live?: boolean }) {
  return (
    <span className={live && n > 0 ? "tab-count live" : "tab-count"} aria-hidden="true">
      {n}
    </span>
  );
}

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [needsLogin, setNeedsLogin] = useState(false);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [live, setLive] = useState(false);
  const [refreshedAt, setRefreshedAt] = useState<string | null>(null);
  // The view comes from the URL (issue #357): the tab, the open issue, the part or seam selected
  // on the map, a new issue being written, and the pages that take the panel's place.
  const route = useRoute();
  const tab = tabOf(route);
  const selectedIssue: IssueRef | null = route.view === "issue" ? route.issue : null;
  const selection = route.sel;
  // "New issue here…": the part or seam a new issue is being written for (ADR 0025).
  // Started from "New issue…" with nothing selected, the place is still to be chosen (null).
  const newIssueAt = route.view === "new-issue" ? { at: route.at } : null;
  // Where the person was before a page took the map's and the panel's place: closing it returns there.
  const returnTo = useRef<Route>(HOME);
  useEffect(() => {
    if (!isPage(route)) {
      returnTo.current = route;
    }
  }, [route]);
  const closePage = useCallback(() => navigate({ ...returnTo.current, sel: currentRoute().sel }), []);
  // Edit mode: the draft panel shows, parts connect, a double-click on the canvas asks for a part.
  const [editing, setEditing] = useState(false);
  const [addPartFocus, setAddPartFocus] = useState(0);
  // A discrepancy taken from the observed system (issue #140): what the draft's form starts with.
  const [prefill, setPrefill] = useState<DraftPrefill | undefined>(undefined);
  const [trafficBusy, setTrafficBusy] = useState(false);
  /** `/api/me` listed no tenant: there is no map to load. */
  const [noTenant, setNoTenant] = useState(false);
  const newPartAt = useRef<{ x: number; y: number } | null>(null);
  // A notice may carry one way back ("Undo"), offered to the person who caused it.
  const [noticeState, setNoticeState] = useState<{ text: string; undo?: (() => void) | undefined } | null>(null);
  const notice = noticeState?.text ?? null;
  const setNotice = useCallback((text: string | null, undo?: () => void) => setNoticeState(text === null ? null : { text, undo }), []);

  const refresh = useCallback(async () => {
    try {
      const ov = await liveApi.overview();
      setOverview(ov);
      const gone = takeForgottenWorkspace();
      if (gone) {
        setNotice(`Workspace ${gone} no longer exists; showing ${ov.map.map.name}`);
      }
      setError(null);
      setRefreshedAt(new Date().toISOString());
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        // `request` raised needs-login; the login page follows.
      } else if (e instanceof ApiError && e.status === 503) {
        setError("The server has no map loaded yet.");
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    }
  }, [setNotice]);

  useEffect(() => {
    // A 401 from any call, or from the probe after the websocket closes, leads here (design S24).
    const offNeedsLogin = onNeedsLogin(() => setNeedsLogin(true));
    // Who the person is comes first: it says which tenants they are in, and every other call
    // goes under one of them (ADR 0089).
    authApi
      .me()
      .then((m) => {
        setMe(m);
        resetNeedsLogin();
        setNeedsLogin(false);
        return learnTenants(m.tenants?.map((t) => t.id));
      })
      .catch((e: unknown) => {
        // A 401 is raised by `request`, and the login page follows: there is no tenant to ask
        // for anything. Any other failure shows through the overview's error.
        return e instanceof ApiError && e.status === 401 ? null : true;
      })
      .then((inATenant) => {
        if (inATenant === null || gone) {
          return;
        }
        if (!inATenant) {
          // An operator need be in no tenant: the operator's page is theirs to start from.
          setNoTenant(true);
          return;
        }
        void refresh();
        // The socket is opened for the tenant just learned (ADR 0089). A change for another
        // workspace is not ours to refetch for (ADR 0049).
        unsubscribe = subscribe(
          (ev) => {
            if (concernsUs(ev)) {
              void refresh();
            }
          },
          setLive,
          // Removed from this tenant: start again from whatever tenants are left.
          () => {
            forgetTenant();
            // What was open belongs to the tenant that is gone.
            navigate(HOME, { replace: true });
            location.reload();
          },
        );
      });
    let gone = false;
    let unsubscribe = () => {};
    return () => {
      gone = true;
      unsubscribe();
      offNeedsLogin();
    };
  }, [refresh]);

  /** Adds a pin: from a drop on the map or from a row's "Pin to…" menu. A refusal shows the server's reason. */
  const onPin = useCallback(
    async (issue: IssueRef, target: MapRef) => {
      try {
        await pinsApi.addPin({ issue, target });
        setNotice(`${issueKey(issue)} pinned to ${target.id}`);
        go({ view: "issue", issue, section: "overview", from: null });
        await refresh();
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`${issueKey(issue)} was not pinned to ${target.id}: ${failureText(e)}`);
      }
    },
    [refresh, setNotice],
  );

  /** Parts were dragged, or moved with the arrow keys: saved for the whole team (ADR 0016). */
  const onMove = useCallback(
    async (moved: MapLayout) => {
      try {
        await mapApi.putLayout(moved);
        await refresh();
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`The new position of ${[...Object.keys(moved.positions ?? {}), ...Object.keys(moved.grouped ?? {}), ...Object.keys(moved.boxes ?? {})].join(", ")} was not saved: ${failureText(e)}`);
      }
    },
    [refresh, setNotice],
  );

  const restoreLayout = useCallback(
    async (removed: MapLayout) => {
      try {
        await mapApi.putLayout(removed);
        await refresh();
        setNotice("The saved positions are back");
      } catch (e) {
        setNotice(`The saved positions were not restored: ${failureText(e)}`);
      }
    },
    [refresh, setNotice],
  );

  /** Auto-arrange the flat map, or only the By repo arrangement (ADR 0068); Undo puts back what was removed. */
  const onAutoArrange = useCallback(async (arrangement: "flat" | "grouped") => {
    try {
      const removed = arrangement === "grouped" ? await mapApi.clearGrouped() : await mapApi.clearLayout();
      await refresh();
      const n = Object.keys(removed.positions ?? {}).length + Object.keys(removed.grouped ?? {}).length + Object.keys(removed.boxes ?? {}).length;
      const what = arrangement === "grouped" ? "By repo auto-arranged for everyone" : "Auto-arranged for everyone";
      setNotice(`${what}: ${n} saved position${n === 1 ? "" : "s"} removed`, () => void restoreLayout(removed));
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        return;
      }
      setNotice(`The map was not auto-arranged: ${failureText(e)}`);
    }
  }, [refresh, restoreLayout, setNotice]);

  /** Queue an issue on one of the caller's own lanes: a drop on the lane, or "Queue on…". */
  const onQueue = useCallback(
    async (issue: IssueRef, companionId: string) => {
      const lane = overview?.companions.find((c) => c.companion_id === companionId)?.hostname ?? "your companion";
      try {
        await queueApi.add({ issue, companion_id: companionId });
        setNotice(`${issueKey(issue)} queued on ${lane}`);
        await refresh();
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`${issueKey(issue)} was not queued on ${lane}: ${failureText(e)}`);
      }
    },
    [overview, refresh, setNotice],
  );

  const onReorder = useCallback(
    async (companionId: string, issues: IssueRef[]) => {
      try {
        await queueApi.reorder(companionId, issues);
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`The queue was not reordered: ${failureText(e)}`);
      }
      // Also after a refusal: the lane shown is then the lane as it is.
      await refresh();
    },
    [refresh, setNotice],
  );

  const onUnqueue = useCallback(
    async (issue: IssueRef, companionId: string) => {
      try {
        await queueApi.remove({ issue, companion_id: companionId });
        // Removing is undone by queueing it again, at the end of the lane.
        setNotice(`${issueKey(issue)} removed from the queue`, () => void onQueue(issue, companionId));
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`${issueKey(issue)} was not removed from the queue: ${failureText(e)}`);
      }
      await refresh();
    },
    [refresh, setNotice, onQueue],
  );

  /** Start from the queue: only the issue and the lane's companion are sent; the server resolves the caps. */
  const onStart = useCallback(
    async (issue: IssueRef, companionId: string) => {
      try {
        const r = await launchApi.launch({ issue, companion_id: companionId });
        setNotice(`${launchNotice(r, issue)} It leaves the queue when the companion reports it started.`);
        await refresh();
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`No session was started for ${issueKey(issue)}; it stays queued: ${launchFailureText(e)}`);
      }
    },
    [refresh, setNotice],
  );

  /** A part or seam chosen on the map: its inspector shows, and an open issue gives way. */
  const onSelectOnMap = useCallback((ref: MapRef | null) => {
    if (ref) {
      navigate({ view: "map", sel: ref });
    } else if (currentRoute().view === "map") {
      // Nothing selected leaves the inspector nothing to show.
      navigate(HOME, { replace: true });
    } else {
      selectOnly(null);
    }
  }, []);

  // Escape closes what is open in the panel, innermost first: a form, an issue, then the selection.
  // Not while typing, and not when a menu or dialog already took the key.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) {
        return;
      }
      const t = e.target;
      if (t instanceof Element && (t.closest("input, textarea, select, [contenteditable=\"true\"], [role=\"menu\"], dialog") !== null)) {
        return;
      }
      // Up one level, never Back: a pasted link has no history to go back to. A page that took
      // the panel's place closes to where the person was.
      const here = currentRoute();
      const up = isPage(here) ? { ...returnTo.current, sel: here.sel } : parentOf(here);
      if (!up) {
        return;
      }
      navigate(up);
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const onUnpin = async (req: PinRequest) => {
    try {
      await pinsApi.removePin(req);
      await refresh();
    } catch (e) {
      setNotice(`${issueKey(req.issue)} was not unpinned from ${req.target.id}: ${failureText(e)}`);
    }
  };

  /** Start your own lead agent (ADR 0072): only offered while the server's orchestrator flag is on. */
  const onLaunchLead = async (issue: IssueRef, companionId: string) => {
    try {
      const r = await launchApi.launch({ issue, companion_id: companionId, role: "lead" });
      setNotice(`${launchNotice(r, issue)} It leads only the workers it launches for you.`);
      await refresh();
    } catch (e) {
      setNotice(`No lead agent was started for ${issueKey(issue)}: ${launchFailureText(e)}`);
    }
  };

  /** Start a session; `branch` resumes one of your drafts on it (ADR 0053). */
  const onLaunch = async (issue: IssueRef, branch?: string) => {
    try {
      const r = await launchApi.launch(branch ? { issue, branch } : { issue });
      setNotice(launchNotice(r, issue));
      await refresh();
    } catch (e) {
      setNotice(`No session was started for ${issueKey(issue)}: ${launchFailureText(e)}`);
    }
  };

  /** A seam drawn on the map in edit mode becomes an operation of the draft; nothing is drawn until the server has it. */
  const onConnectParts = useCallback(
    async (from: string, to: string) => {
      const op = seamOpFor(from, to);
      if (!op) {
        return;
      }
      try {
        await draftsApi.addOp(op);
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) {
          return;
        }
        setNotice(`The seam from ${from} to ${to} was not added to the draft: ${failureText(e)}`);
      }
      await refresh();
    },
    [refresh, setNotice],
  );

  /** A part added after a double-click on the canvas is placed where the canvas was clicked. */
  const onPartAdded = useCallback(
    async (id: string) => {
      const at = newPartAt.current;
      newPartAt.current = null;
      if (!at) {
        return;
      }
      try {
        await mapApi.putLayout({ positions: { [id]: at } });
        await refresh();
      } catch {
        // The part is in the draft; where it sits is the automatic layout's call.
      }
    },
    [refresh],
  );

  /**
   * "Take into draft" (issue #140): edit mode opens with the form filled in from the telemetry,
   * or the quiet seam selected so its Remove is at hand. Nothing is sent: the person submits.
   */
  const onTake = useCallback((take: Take) => {
    if (take.kind === "add_node") {
      setPrefill({ nonce: Date.now(), op: "add_node", id: take.id, kind: take.nodeKind, service: take.service });
    } else if (take.kind === "add_seam") {
      setPrefill({ nonce: Date.now(), op: "add_seam", from: take.from, to: take.to, id: take.id });
    } else if (take.kind === "select_seam") {
      setPrefill(undefined);
      selectOnly({ kind: "seam", id: take.seam });
    } else {
      return;
    }
    setEditing(true);
  }, []);

  /** Dismiss a discrepancy for the workspace as yourself (ADR 0060), with a way back. */
  const onDismiss = useCallback(
    async (key: DiscrepancyKey) => {
      setTrafficBusy(true);
      try {
        const d = await mapApi.dismiss(key);
        setNotice("Dismissed for everyone in this workspace.", () => {
          setNotice(null);
          void mapApi.undismiss(d.id).then(refresh, (e: unknown) => setNotice(`The dismissal was not undone: ${failureText(e)}`));
        });
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 401)) {
          setNotice(`Not dismissed: ${failureText(e)}`);
        }
      } finally {
        setTrafficBusy(false);
        await refresh();
      }
    },
    [refresh, setNotice],
  );

  const onUndismiss = useCallback(
    async (id: number) => {
      setTrafficBusy(true);
      try {
        await mapApi.undismiss(id);
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 401)) {
          setNotice(`The dismissal was not undone: ${failureText(e)}`);
        }
      } finally {
        setTrafficBusy(false);
        await refresh();
      }
    },
    [refresh, setNotice],
  );

  /** Show another workspace: the switcher, a workspace just created, or `default` after a delete. */
  const switchWorkspace = useCallback(
    (id: string) => {
      setWorkspace(id);
      // An issue or a part of one workspace means nothing in another; the settings stay open on
      // the workspace just chosen.
      navigate(currentRoute().view === "settings" ? { view: "settings", sel: null } : HOME);
      returnTo.current = HOME;
      setEditing(false);
      void refresh();
    },
    [refresh],
  );

  // The draft is an overlay for everyone: what it adds and removes is drawn, marked as draft.
  const shown = useMemo(() => (overview ? withDraft(overview) : null), [overview]);
  const marks = useMemo(() => (overview ? draftMarks(overview.map.map, overview.draft) : undefined), [overview]);
  const graph = useMemo(() => (shown ? buildGraph(shown) : null), [shown]);

  // The stored size applies only while the map and the panel sit side by side.
  const sideBySide = useSideBySide();
  const [storedPanelSize, setStoredPanelSize] = useState<PanelSize>(readPanelSize);
  const panelSize: PanelSize = sideBySide ? storedPanelSize : "normal";

  // After every hook: the login page replaces the whole shell.
  if (needsLogin) {
    return <LoginPage />;
  }

  const selected = overview && selectedIssue ? (overview.issues.find((i) => issueKey(i.issue) === issueKey(selectedIssue)) ?? null) : null;

  // A link may name what is not here: an issue the overview does not hold, or nothing at all.
  // Said only once the overview is in, so a slow start never reads as "not found".
  const missing = !overview
    ? null
    : route.view === "not-found"
      ? `Nothing is at #${route.fragment}.`
      : selectedIssue && !selected
        ? `${issueKey(selectedIssue)} is not among the issues Rubato tracks in this workspace.`
        : null;

  /** The tenant the page shows, with the person's own role in it. */
  const myTenant = me?.tenants?.find((t) => t.id === currentTenant()) ?? null;
  /** An operator in no tenant has one page: their own (ADR 0088). */
  const operatorOnly = noTenant && me?.operator === true;
  const page = isPage(route) || operatorOnly;
  /** The views under the Issues tab that are about issues: the list, an issue, a link that named nothing. */
  const onIssues = route.view === "issues" || route.view === "issue" || route.view === "not-found";
  /** The plain issue list: nothing open in its place. */
  const list = route.view === "issues" || (onIssues && !selected);

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <a className="brand-name" href="#/issues" title="The issue list">
            <strong>Rubato</strong>
          </a>
          {overview && <WorkspaceMenu overview={overview} onSwitch={switchWorkspace} onConfigure={() => go({ view: "settings" })} onWorkspaces={() => go({ view: "workspaces" })} />}
        </div>
        <div className="status">
          {/* A signal that stopped must not look live: the words show whenever it has. */}
          <span className={live ? "dot live" : "dot"} data-testid="live-dot" role="img" aria-label={live ? "live" : "disconnected"} title={`${live ? "live" : "disconnected"}${refreshedAt ? ` · refreshed ${relative(refreshedAt)}` : ""}`} />
          {!live && (
            <span className="muted" data-testid="live-words">
              disconnected{refreshedAt ? ` · refreshed ${relative(refreshedAt)}` : ""}
            </span>
          )}
          {me && (
            <AccountMenu
              me={me}
              tenant={currentTenant()}
              onSwitchTenant={(id) => {
                // Another tenant is another map, other people and another socket: the page starts over.
                navigate(HOME, { replace: true });
                setTenant(id);
                location.reload();
              }}
              onMembers={myTenant ? () => go({ view: "members" }) : undefined}
              onOperator={me.operator ? () => go({ view: "operator" }) : undefined}
              onNotice={(text) => setNotice(text)}
            />
          )}
        </div>
      </header>
      {error && <div className="banner error">{error}</div>}
      {noTenant && !error && (
        <div className="banner notice" data-testid="no-tenant">
          {me?.operator ? "You are in no tenant. As an operator you can create one and name its admin; a tenant's data is its members' to see." : "You are not in a tenant yet. Ask one of its admins for an invite."}
        </div>
      )}
      {/* The live region stays mounted so a screen reader hears each notice; the button sits outside it. */}
      <div className={notice ? "banner notice" : undefined}>
        <span role="status">{notice}</span>
        {noticeState?.undo && (
          <button type="button" onClick={noticeState.undo}>
            Undo
          </button>
        )}
        {notice && (
          <button type="button" onClick={() => setNotice(null)}>
            Dismiss
          </button>
        )}
      </div>
      <div className="body" data-panel={panelSize} data-page={page ? route.view : undefined} data-testid="body">
        {/* Hidden at full size and under a page, so the map lays nothing out at no width and leaves the tab order; it stays mounted and keeps its lens. */}
        <section className="map" hidden={panelSize === "full" || page}>
          {graph && overview ? (
            <MapView graph={graph} selected={selection} onSelect={onSelectOnMap} onPin={onPin} layout={overview.layout} onMove={onMove} onAutoArrange={(arrangement) => void onAutoArrange(arrangement)} plan={selected?.plan ? { issue: selected.issue, plan: selected.plan } : null}
              draftMarks={marks}
              editing={editing}
              onToggleEdit={() => setEditing((e) => !e)}
              onConnectParts={(from, to) => void onConnectParts(from, to)}
              onAddPartAt={(at) => {
                newPartAt.current = at;
                setAddPartFocus((n) => n + 1);
              }}
              onConfigure={() => go({ view: "settings" })}
              trafficActions={<Discrepancies overview={overview} onTake={onTake} onDismiss={(k) => void onDismiss(k)} onUndo={(id) => void onUndismiss(id)} busy={trafficBusy} canWrite={me !== null} />}
            />
          ) : (
            <div className="empty">{error ?? (noTenant ? "No tenant, so no map to show." : "Loading the map…")}</div>
          )}
        </section>
        {page ? (
          <main className="page" data-view={operatorOnly ? "operator" : route.view}>
            {route.view === "members" && myTenant && me ? (
              <MembersView tenant={myTenant} me={me.login} onClose={closePage} onNotice={(text) => setNotice(text)} />
            ) : route.view === "operator" || operatorOnly ? (
              <AdminView onClose={closePage} onNotice={(text) => setNotice(text)} />
            ) : route.view === "workspaces" && overview ? (
              <WorkspacesView overview={overview} onClose={closePage} onChanged={() => void refresh()} onNotice={(text) => setNotice(text)} onSwitch={switchWorkspace} />
            ) : route.view === "settings" && overview ? (
              <SettingsView overview={overview} onClose={closePage} onChanged={() => void refresh()} onNotice={(text) => setNotice(text)} />
            ) : (
              <p className="muted">{error ?? "Loading…"}</p>
            )}
          </main>
        ) : (
        <aside className="panel">
          <nav className="tabs">
            <button className={tab === "issues" ? "active" : ""} onClick={() => go({ view: "issues", list: "open" })} title="Open issues Rubato tracks: pinned to the map, or with a plan">
              Issues
            </button>
            <button className={tab === "plans" ? "active" : ""} onClick={() => go({ view: "plans", list: "plans" })} data-testid="tab-plans" aria-label={overview ? `Plans (${groupPlans(overview).proposed.length} proposed)` : undefined} title="Plans waiting for approval are counted">
              Plans{overview && <TabCount n={groupPlans(overview).proposed.length} />}
            </button>
            {/* Dragging an issue over this tab opens it, so the issue can be dropped on a lane. */}
            <button
              className={tab === "companions" ? "active" : ""}
              onClick={() => go({ view: "companions" })}
              aria-label={overview ? `Companions (${overview.companions.filter((c) => c.connected).length} live)` : undefined}
              title="Connected companions are counted"
              onDragEnter={(e) => {
                if (hasIssueDrag(e)) {
                  // Part of one gesture, so not a step for Back to undo.
                  go({ view: "companions" }, { replace: true });
                }
              }}
            >
              Companions{overview && <TabCount n={overview.companions.filter((c) => c.connected).length} live />}
            </button>
            <button className={tab === "repos" ? "active" : ""} onClick={() => go({ view: "repos" })} data-testid="tab-repos">
              Repos
            </button>
            {/* One button, three sizes (#265). Stacked under 901 px, the panel has one size. */}
            {sideBySide && (
              <button
                type="button"
                className="panel-size"
                data-testid="panel-size"
                aria-label={PANEL_ACTION[panelSize].name}
                title={PANEL_ACTION[panelSize].name}
                onClick={() => {
                  const next = nextPanelSize(panelSize);
                  setStoredPanelSize(next);
                  storePanelSize(next);
                }}
              >
                {PANEL_ACTION[panelSize].word}
              </button>
            )}
          </nav>
          <div className="view" data-view={route.view}>
          {overview && route.view === "findings" && (
            <section className="findings-view" aria-label="Findings">
              <p>
                <button type="button" onClick={goUp}>
                  ← all issues
                </button>
              </p>
              <h2>What the map and the plans disagree about</h2>
              {overview.findings.length > 0 ? <Findings findings={overview.findings} /> : <p className="muted">Nothing: the map, the plans and what is observed agree.</p>}
            </section>
          )}
          {overview && missing && (
            <p className="banner notice not-found" data-testid="not-found">
              {missing}{" "}
              <button type="button" onClick={goUp}>
                {tab === "plans" ? "All plans" : "All issues"}
              </button>
            </p>
          )}
          {overview && editing && <DraftPanel overview={overview} selection={selection} onChanged={refresh} onNotice={setNotice} focusAddPart={addPartFocus} onPartAdded={(id) => void onPartAdded(id)} prefill={prefill} />}
          {/* The map explains what is selected; an open issue takes the panel instead. */}
          {/* With a part or seam selected, its inspector offers "New issue here…" instead. */}
          {overview && list && !selection && <TeamTrack issues={openIssues(overview)} />}
          {overview && route.view === "issues" && route.list === "open" && !selection && liveMap(overview.map.map).nodes[0] && (
            <p className="new-issue-start">
              <button type="button" data-testid="new-issue-start" onClick={() => go({ view: "new-issue", at: null })}>
                New issue…
              </button>
            </p>
          )}
          {overview && newIssueAt && (
            <NewIssueForm
              key={newIssueAt.at ? `${newIssueAt.at.kind}:${newIssueAt.at.id}` : "anywhere"}
              overview={overview}
              target={newIssueAt.at}
              onCancel={goUp}
              onNotice={(text) => setNotice(text)}
              onCreated={(issue) => {
                // The overview has to hold the issue before its view can show it.
                void refresh().then(() => go({ view: "issue", issue, section: "overview", from: null }, { replace: true }));
              }}
            />
          )}
          {overview && route.view === "map" && selection && (
            <Inspector
              overview={shown ?? overview}
              selection={selection}
              onSelect={onSelectOnMap}
              onSelectIssue={(issue) => go({ view: "issue", issue, section: "overview", from: null })}
              onNewIssue={(at) => go({ view: "new-issue", at })}
            />
          )}
          {overview && list && (
            <IssueList
              overview={overview}
              me={me}
              list={route.view === "issues" ? route.list : "open"}
              onList={(which) => go({ view: "issues", list: which })}
              selection={selection}
              onClearSelection={() => selectOnly(null)}
              onOpen={(issue) => go({ view: "issue", issue, section: "overview", from: null })}
              onPin={onPin}
              onQueue={onQueue}
            />
          )}
          {overview && route.view === "issue" && selected && (
            <IssuePanel
              overview={overview}
              me={me}
              selected={selected}
              section={route.section}
              onSection={(section) => go({ view: "issue", issue: selected.issue, section, from: route.from })}
              // The back button says where it goes: the list, or the board the plan was opened from
              // (Escape goes up, to the part's inspector when one is selected).
              backLabel={route.from ? "← all plans" : "← all issues"}
              onClose={() => go(route.from ? { view: "plans", list: route.from } : { view: "issues", list: "open" })}
              onOpenIssue={(issue) => go({ view: "issue", issue, section: "overview", from: null })}
              planOnMap={selected.plan ? <PlanInspector issue={selected} onSelect={onSelectOnMap} showDecisions={false} /> : null}
              onLaunch={onLaunch}
              onLaunchLead={onLaunchLead}
              onPin={onPin}
              onUnpin={onUnpin}
              onNotice={(text) => {
                setNotice(text);
                void refresh();
              }}
              onShowSeam={(id) => onSelectOnMap({ kind: "seam", id })}
              onPairCompanion={() => go({ view: "companions" })}
            />
          )}
          {overview && route.view === "plans" && (
            <PlansBoard overview={overview} done={route.list === "plans-done"} onDone={(done) => go({ view: "plans", list: done ? "plans-done" : "plans" })} onOpen={(issue) => go({ view: "issue", issue, section: "plan", from: route.list })} />
          )}
          {overview && route.view === "repos" && <ConnectedRepos overview={overview} onOpenRepo={(repo) => go({ view: "repo", repo })} />}
          {overview && route.view === "repo" && (
            <RepoView
              overview={overview}
              repo={route.repo}
              onBack={() => go({ view: "repos" })}
              onOpenIssue={(issue) => go({ view: "issue", issue, section: "overview", from: null })}
              onOpenPull={(number) => go({ view: "pull", repo: route.repo, number })}
              onPin={onPin}
            />
          )}
          {overview && route.view === "pull" && (
            <section data-testid="pull-page" aria-label={`Pull request ${route.repo}#${route.number}`}>
              <p>
                <button type="button" onClick={() => go({ view: "repo", repo: route.repo })}>
                  ← {route.repo}
                </button>
              </p>
              <PullView repo={route.repo} number={route.number} refreshKey={overview.generated_at} onOpenIssue={(issue) => go({ view: "issue", issue, section: "overview", from: null })} />
            </section>
          )}
          {overview && tab === "companions" && <CompanionsPanel overview={overview} me={me} onChanged={refresh} onQueue={onQueue} onReorder={onReorder} onUnqueue={onUnqueue} onStart={onStart} />}
          </div>
          {/* Quiet, and only when there is something: what the map and the plans disagree about has its own view. */}
          {overview && overview.findings.length > 0 && route.view !== "findings" && (
            <p className="findings-link">
              <a href="#/findings" data-testid="findings-link" onClick={(e) => { e.preventDefault(); go({ view: "findings" }); }}>
                {overview.findings.length} finding{overview.findings.length === 1 ? "" : "s"}
              </a>
            </p>
          )}
        </aside>
        )}
      </div>
    </div>
  );
}
