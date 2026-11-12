import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LoginPage, authApi } from "./features/auth/index.ts";
import { CompanionsPanel } from "./features/companions/index.ts";
import { DraftPanel, draftMarks, draftsApi, seamOpFor, withDraft } from "./features/drafts/index.ts";
import { Findings } from "./features/findings/index.ts";
import { IssuePanel } from "./features/issues/index.ts";
import { launchApi, launchNotice } from "./features/launch/index.ts";
import { liveApi, subscribe } from "./features/live/index.ts";
import { NewIssueForm } from "./features/new_issue/index.ts";
import { Inspector, MapView, PlanInspector, buildGraph, mapApi } from "./features/map/index.ts";
import { pinsApi } from "./features/pins/index.ts";
import { queueApi } from "./features/queue/index.ts";
import type { IssueRef, MapLayout, MapRef, Me, Overview, PinRequest } from "./protocol.ts";
import { ApiError, failureText, onNeedsLogin, resetNeedsLogin } from "./shared/api.ts";
import { hasIssueDrag } from "./shared/dnd.ts";
import { issueKey, relative } from "./shared/format.ts";

type Tab = "issues" | "companions";

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
  const [newIssueAt, setNewIssueAt] = useState<MapRef | null>(null);
  const [tab, setTab] = useState<Tab>("issues");
  // Edit mode: the draft panel shows, parts connect, a double-click on the canvas asks for a part.
  const [editing, setEditing] = useState(false);
  const [addPartFocus, setAddPartFocus] = useState(0);
  const newPartAt = useRef<{ x: number; y: number } | null>(null);
  // A notice may carry one way back ("Undo"), offered to the person who caused it.
  const [noticeState, setNoticeState] = useState<{ text: string; undo?: (() => void) | undefined } | null>(null);
  const notice = noticeState?.text ?? null;
  const setNotice = useCallback((text: string | null, undo?: () => void) => setNoticeState(text === null ? null : { text, undo }), []);

  const refresh = useCallback(async () => {
    try {
      const ov = await liveApi.overview();
      setOverview(ov);
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
  }, []);

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
    const unsubscribe = subscribe(() => void refresh(), setLive);
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
        setNotice(`The new position of ${Object.keys(moved.positions ?? {}).join(", ")} was not saved: ${failureText(e)}`);
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

  const onAutoArrange = useCallback(async () => {
    try {
      const removed = await mapApi.clearLayout();
      await refresh();
      const n = Object.keys(removed.positions ?? {}).length;
      setNotice(`Auto-arranged for everyone: ${n} saved position${n === 1 ? "" : "s"} removed`, () => void restoreLayout(removed));
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
        setNotice(`No session was started for ${issueKey(issue)}; it stays queued: ${failureText(e)}`);
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

  const onUnpin = async (req: PinRequest) => {
    try {
      await pinsApi.removePin(req);
      await refresh();
    } catch (e) {
      setNotice(`${issueKey(req.issue)} was not unpinned from ${req.target.id}: ${failureText(e)}`);
    }
  };

  const onLaunch = async (issue: IssueRef) => {
    try {
      const r = await launchApi.launch({ issue });
      setNotice(launchNotice(r, issue));
      await refresh();
    } catch (e) {
      setNotice(`No session was started for ${issueKey(issue)}: ${failureText(e)}`);
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

  // The draft is an overlay for everyone: what it adds and removes is drawn, marked as draft.
  const shown = useMemo(() => (overview ? withDraft(overview) : null), [overview]);
  const marks = useMemo(() => (overview ? draftMarks(overview.map.map, overview.draft) : undefined), [overview]);
  const graph = useMemo(() => (shown ? buildGraph(shown) : null), [shown]);

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
          {overview && <span className="muted">map {overview.map.map.name} @ {overview.map.version}</span>}
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
      <div className="body">
        <section className="map">
          {graph && overview ? (
            <MapView graph={graph} selected={selection} onSelect={onSelectOnMap} onPin={onPin} layout={overview.layout} onMove={onMove} onAutoArrange={() => void onAutoArrange()} plan={selected?.plan ? { issue: selected.issue, plan: selected.plan } : null}
              draftMarks={marks}
              editing={editing}
              onToggleEdit={() => setEditing((e) => !e)}
              onConnectParts={(from, to) => void onConnectParts(from, to)}
              onAddPartAt={(at) => {
                newPartAt.current = at;
                setAddPartFocus((n) => n + 1);
              }}
            />
          ) : (
            <div className="empty">{error ?? "Loading the map…"}</div>
          )}
        </section>
        <aside className="panel">
          <nav className="tabs">
            <button className={tab === "issues" ? "active" : ""} onClick={() => setTab("issues")}>
              Issues{overview ? ` (${overview.issues.length})` : ""}
            </button>
            {/* Dragging an issue over this tab opens it, so the issue can be dropped on a lane. */}
            <button
              className={tab === "companions" ? "active" : ""}
              onClick={() => setTab("companions")}
              onDragEnter={(e) => {
                if (hasIssueDrag(e)) {
                  setTab("companions");
                }
              }}
            >
              Companions{overview ? ` (${overview.companions.filter((c) => c.connected).length} live)` : ""}
            </button>
          </nav>
          {overview && <Findings findings={overview.findings} />}
          {overview && editing && <DraftPanel overview={overview} selection={selection} onChanged={refresh} onNotice={setNotice} focusAddPart={addPartFocus} onPartAdded={(id) => void onPartAdded(id)} />}
          {/* The map explains what is selected; an open issue takes the panel instead. */}
          {overview && tab === "issues" && !newIssueAt && !selected && overview.map.map.nodes[0] && (
            <p className="new-issue-start">
              <button type="button" data-testid="new-issue-start" onClick={() => setNewIssueAt(selection ?? { kind: "node", id: overview.map.map.nodes[0]?.id ?? "" })}>
                New issue…
              </button>
            </p>
          )}
          {overview && tab === "issues" && newIssueAt && (
            <NewIssueForm
              key={`${newIssueAt.kind}:${newIssueAt.id}`}
              overview={overview}
              target={newIssueAt}
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
              onNewIssue={setNewIssueAt}
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
              onPin={onPin}
              onUnpin={onUnpin}
              onQueue={onQueue}
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
        </aside>
      </div>
    </div>
  );
}
