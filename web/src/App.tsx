import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LoginPage, authApi } from "./features/auth/index.ts";
import { CompanionsPanel } from "./features/companions/index.ts";
import { DraftPanel, type DraftPrefill, draftMarks, draftsApi, seamOpFor, withDraft } from "./features/drafts/index.ts";
import { Findings } from "./features/findings/index.ts";
import { IssuePanel, TeamTrack, ownConnectedCompanions } from "./features/issues/index.ts";
import { launchApi, launchFailureText, launchNotice } from "./features/launch/index.ts";
import { liveApi, subscribe, takeForgottenWorkspace } from "./features/live/index.ts";
import { NewIssueForm } from "./features/new_issue/index.ts";
import { Discrepancies, Inspector, MapView, PlanInspector, type Take, buildGraph, mapApi } from "./features/map/index.ts";
import { pinsApi } from "./features/pins/index.ts";
import { PlansBoard, groupPlans } from "./features/plans/index.ts";
import { ConnectedRepos } from "./features/repos/index.ts";
import { queueApi } from "./features/queue/index.ts";
import { SettingsView } from "./features/settings/index.ts";
import { WorkspaceSwitcher } from "./features/workspaces/index.ts";
import type { DiscrepancyKey, IssueRef, MapLayout, MapRef, Me, Overview, PinRequest } from "./protocol.ts";
import { ApiError, failureText, onNeedsLogin, resetNeedsLogin } from "./shared/api.ts";
import { hasIssueDrag } from "./shared/dnd.ts";
import { openIssues } from "./shared/finished.ts";
import { issueKey, relative } from "./shared/format.ts";
import { concernsUs, setWorkspace } from "./shared/workspace.ts";
import { liveMap } from "./shared/map.ts";
import { PANEL_ACTION, type PanelSize, nextPanelSize, readPanelSize, storePanelSize, useSideBySide } from "./shared/panel.ts";

type Tab = "issues" | "plans" | "companions" | "repos";

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
  const [selectedIssue, setSelectedIssue] = useState<IssueRef | null>(null);
  const [selection, setSelection] = useState<MapRef | null>(null);
  // "New issue here…": the part or seam a new issue is being written for (ADR 0025).
  // Started from "New issue…" with nothing selected, the place is still to be chosen (null).
  const [newIssueAt, setNewIssueAt] = useState<{ at: MapRef | null } | null>(null);
  const [tab, setTab] = useState<Tab>("issues");
  // Edit mode: the draft panel shows, parts connect, a double-click on the canvas asks for a part.
  const [editing, setEditing] = useState(false);
  const [addPartFocus, setAddPartFocus] = useState(0);
  // A discrepancy taken from the observed system (issue #140): what the draft's form starts with.
  const [prefill, setPrefill] = useState<DraftPrefill | undefined>(undefined);
  const [trafficBusy, setTrafficBusy] = useState(false);
  // Configure workspace (issue #189): the panel shows this workspace's settings instead of the tabs.
  const [configuring, setConfiguring] = useState(false);
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
    authApi
      .me()
      .then((m) => {
        setMe(m);
        resetNeedsLogin();
        setNeedsLogin(false);
      })
      .catch(() => {
        // A 401 is raised by `request`; any other failure shows through the overview's error.
      });
    void refresh();
    // A change for another workspace is not ours to refetch for (ADR 0049).
    const unsubscribe = subscribe((ev) => {
      if (concernsUs(ev)) {
        void refresh();
      }
    }, setLive);
    return () => {
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
        setTab("issues");
        setSelectedIssue(issue);
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

  const onSelectOnMap = useCallback((ref: MapRef | null) => {
    setSelection(ref);
    if (ref) {
      setTab("issues");
      setSelectedIssue(null);
    }
  }, []);

  // Escape closes what is open in the panel, innermost first: a form, an issue, then the selection.
  // Not while typing, and not when a menu or dialog already took the key.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) {
        return;
      }
      const t = e.target as HTMLElement | null;
      if (t && (t.closest("input, textarea, select, [contenteditable=\"true\"], [role=\"menu\"], dialog") !== null)) {
        return;
      }
      if (newIssueAt) {
        setNewIssueAt(null);
      } else if (selectedIssue) {
        setSelectedIssue(null);
      } else if (selection) {
        setSelection(null);
      } else {
        return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newIssueAt, selectedIssue, selection]);

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
      setSelection({ kind: "seam", id: take.seam });
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
      setSelection(null);
      setSelectedIssue(null);
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

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <strong>Rubato</strong>
          {overview && <span className="map-version" title={`The map file as read at commit ${overview.map.version}; plans name parts of this version`}>map {overview.map.map.name} @ {overview.map.version.slice(0, 7)}</span>}
          {overview && <WorkspaceSwitcher overview={overview} onSwitch={switchWorkspace} />}
          {/* Always drawn, also with one workspace, where the switcher is not (ADR 0075). */}
          {overview && (
            <button type="button" className="configure-workspace" data-testid="configure-workspace" aria-pressed={configuring} onClick={() => setConfiguring((c) => !c)}>
              Configure workspace
            </button>
          )}
          {overview && <TeamTrack issues={openIssues(overview)} />}
        </div>
        <div className="status">
          <span className={live ? "dot live" : "dot"} title={live ? "live" : "disconnected"} />
          <span className="muted">{live ? "live" : "disconnected"}{refreshedAt ? ` · refreshed ${relative(refreshedAt)}` : ""}</span>
          {me && <span className="me">{me.login}</span>}
        </div>
      </header>
      {error && <div className="banner error">{error}</div>}
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
      <div className="body" data-panel={panelSize} data-testid="body">
        {/* Hidden at full size, so the map lays nothing out at no width and leaves the tab order. */}
        <section className="map" hidden={panelSize === "full"}>
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
              onConfigure={() => setConfiguring(true)}
              trafficActions={<Discrepancies overview={overview} onTake={onTake} onDismiss={(k) => void onDismiss(k)} onUndo={(id) => void onUndismiss(id)} busy={trafficBusy} canWrite={me !== null} />}
            />
          ) : (
            <div className="empty">{error ?? "Loading the map…"}</div>
          )}
        </section>
        <aside className="panel">
          {configuring && overview ? (
            <SettingsView overview={overview} onClose={() => setConfiguring(false)} onChanged={() => void refresh()} onNotice={(text) => setNotice(text)} onSwitch={switchWorkspace} />
          ) : (
          <>
          <nav className="tabs">
            <button className={tab === "issues" ? "active" : ""} onClick={() => setTab("issues")} aria-label={overview ? `Issues (${openIssues(overview).length})` : undefined} title="Open issues Rubato tracks: pinned to the map, or with a plan">
              Issues{overview && <TabCount n={openIssues(overview).length} />}
            </button>
            <button className={tab === "plans" ? "active" : ""} onClick={() => setTab("plans")} data-testid="tab-plans" aria-label={overview ? `Plans (${groupPlans(overview).proposed.length} proposed)` : undefined} title="Plans waiting for approval are counted">
              Plans{overview && <TabCount n={groupPlans(overview).proposed.length} />}
            </button>
            {/* Dragging an issue over this tab opens it, so the issue can be dropped on a lane. */}
            <button
              className={tab === "companions" ? "active" : ""}
              onClick={() => setTab("companions")}
              aria-label={overview ? `Companions (${overview.companions.filter((c) => c.connected).length} live)` : undefined}
              title="Connected companions are counted"
              onDragEnter={(e) => {
                if (hasIssueDrag(e)) {
                  setTab("companions");
                }
              }}
            >
              Companions{overview && <TabCount n={overview.companions.filter((c) => c.connected).length} live />}
            </button>
            <button className={tab === "repos" ? "active" : ""} onClick={() => setTab("repos")} data-testid="tab-repos" aria-label={overview ? `Repos (${(overview.repos ?? []).length})` : undefined}>
              Repos{overview && <TabCount n={(overview.repos ?? []).length} />}
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
          {overview && <Findings findings={overview.findings} />}
          {overview && editing && <DraftPanel overview={overview} selection={selection} onChanged={refresh} onNotice={setNotice} focusAddPart={addPartFocus} onPartAdded={(id) => void onPartAdded(id)} prefill={prefill} />}
          {/* The map explains what is selected; an open issue takes the panel instead. */}
          {/* With a part or seam selected, its inspector offers "New issue here…" instead. */}
          {overview && tab === "issues" && !newIssueAt && !selected && !selection && liveMap(overview.map.map).nodes[0] && (
            <p className="new-issue-start">
              <button type="button" data-testid="new-issue-start" onClick={() => setNewIssueAt({ at: null })}>
                New issue…
              </button>
            </p>
          )}
          {overview && tab === "issues" && newIssueAt && (
            <NewIssueForm
              key={newIssueAt.at ? `${newIssueAt.at.kind}:${newIssueAt.at.id}` : "anywhere"}
              overview={overview}
              target={newIssueAt.at}
              onCancel={() => setNewIssueAt(null)}
              onNotice={(text) => setNotice(text)}
              onCreated={(issue) => {
                setNewIssueAt(null);
                setSelectedIssue(issue);
                void refresh();
              }}
            />
          )}
          {overview && tab === "issues" && selection && !selected && !newIssueAt && (
            <Inspector
              overview={shown ?? overview}
              selection={selection}
              onSelect={onSelectOnMap}
              onSelectIssue={(issue) => {
                setTab("issues");
                setSelectedIssue(issue);
              }}
              onNewIssue={(at) => setNewIssueAt({ at })}
            />
          )}
          {overview && tab === "issues" && (
            <IssuePanel
              overview={overview}
              me={me}
              selected={selected}
              selection={selection}
              onSelect={setSelectedIssue}
              onClearSelection={() => setSelection(null)}
              onLaunch={onLaunch}
              onLaunchLead={onLaunchLead}
              onPin={onPin}
              onUnpin={onUnpin}
              onQueue={onQueue}
              onNotice={(text) => {
                setNotice(text);
                void refresh();
              }}
              onShowSeam={(id) => onSelectOnMap({ kind: "seam", id })}
              onPairCompanion={() => setTab("companions")}
            />
          )}
          {overview && tab === "plans" && (
            <PlansBoard
              overview={overview}
              me={me}
              selected={selected}
              onSelect={setSelectedIssue}
              onOpenIssue={(issue) => {
                setTab("issues");
                setSelectedIssue(issue);
              }}
              onResume={ownConnectedCompanions(overview, me).length > 0 ? (issue, branch) => onLaunch(issue, branch) : undefined}
              onNotice={(text) => {
                setNotice(text);
                void refresh();
              }}
            />
          )}
          {overview && tab === "repos" && (
            <ConnectedRepos
              overview={overview}
              onPin={onPin}
              onOpenIssue={(issue) => {
                setTab("issues");
                setSelectedIssue(issue);
              }}
            />
          )}
          {/* An open issue with a plan: the map shows the plan, and below the issue this says what the plan says. */}
          {overview && tab === "issues" && selected?.plan && (
            <PlanInspector
              issue={selected}
              onSelect={(ref) => {
                setSelectedIssue(null);
                onSelectOnMap(ref);
              }}
            />
          )}
          {overview && tab === "companions" && <CompanionsPanel overview={overview} me={me} onChanged={refresh} onQueue={onQueue} onReorder={onReorder} onUnqueue={onUnqueue} onStart={onStart} />}
          </>
          )}
        </aside>
      </div>
    </div>
  );
}
