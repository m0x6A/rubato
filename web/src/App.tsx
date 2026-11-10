import { useCallback, useEffect, useState } from "react";
import { LoginPage, authApi } from "./features/auth/index.ts";
import { CompanionsPanel } from "./features/companions/index.ts";
import { Findings } from "./features/findings/index.ts";
import { IssuePanel } from "./features/issues/index.ts";
import { launchApi, launchNotice } from "./features/launch/index.ts";
import { liveApi, subscribe } from "./features/live/index.ts";
import { MapView, buildGraph } from "./features/map/index.ts";
import { PinDialog, pinsApi } from "./features/pins/index.ts";
import type { IssueRef, Me, Overview, PinRequest } from "./protocol.ts";
import { ApiError } from "./shared/api.ts";
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
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("issues");
  const [pinOpen, setPinOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const ov = await liveApi.overview();
      setOverview(ov);
      setError(null);
      setRefreshedAt(new Date().toISOString());
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        setNeedsLogin(true);
      } else if (e instanceof ApiError && e.status === 503) {
        setError("The server has no map loaded yet.");
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    }
  }, []);

  useEffect(() => {
    authApi
      .me()
      .then((m) => {
        setMe(m);
        setNeedsLogin(false);
      })
      .catch((e) => {
        if (e instanceof ApiError && e.status === 401) {
          setNeedsLogin(true);
        }
      });
    void refresh();
    return subscribe(() => void refresh(), setLive);
  }, [refresh]);

  const onPin = async (req: PinRequest) => {
    try {
      await pinsApi.addPin(req);
      setPinOpen(false);
      setSelectedIssue(req.issue);
      await refresh();
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    }
  };

  const onUnpin = async (req: PinRequest) => {
    try {
      await pinsApi.removePin(req);
      await refresh();
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    }
  };

  const onLaunch = async (issue: IssueRef) => {
    try {
      const r = await launchApi.launch({ issue });
      setNotice(launchNotice(r, issue));
      await refresh();
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    }
  };

  if (needsLogin) {
    return <LoginPage />;
  }

  const graph = overview ? buildGraph(overview) : null;
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
      {notice && (
        <div className="banner" onClick={() => setNotice(null)} role="status">
          {notice}
        </div>
      )}
      <div className="body">
        <section className="map">
          {graph && overview ? (
            <MapView
              graph={graph}
              selectedNode={selectedNode}
              onSelectNode={(id) => {
                setSelectedNode(id);
                setTab("issues");
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
            <button className={tab === "companions" ? "active" : ""} onClick={() => setTab("companions")}>
              Companions{overview ? ` (${overview.companions.filter((c) => c.connected).length} live)` : ""}
            </button>
            <button onClick={() => setPinOpen(true)} disabled={!overview}>
              Pin an issue
            </button>
          </nav>
          {overview && <Findings findings={overview.findings} />}
          {overview && tab === "issues" && (
            <IssuePanel
              overview={overview}
              me={me}
              selected={selected}
              selectedNode={selectedNode}
              onSelect={setSelectedIssue}
              onClearNode={() => setSelectedNode(null)}
              onLaunch={onLaunch}
              onUnpin={onUnpin}
            />
          )}
          {overview && tab === "companions" && <CompanionsPanel companions={overview.companions} me={me} onChanged={refresh} />}
        </aside>
      </div>
      {pinOpen && overview && <PinDialog map={overview.map.map} onSubmit={onPin} onClose={() => setPinOpen(false)} />}
    </div>
  );
}
