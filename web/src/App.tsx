import { useCallback, useEffect, useState } from "react";
import { ApiError, api, subscribe } from "./api.ts";
import { CompanionsPanel } from "./components/CompanionsPanel.tsx";
import { Findings } from "./components/Findings.tsx";
import { IssuePanel } from "./components/IssuePanel.tsx";
import { MapView } from "./components/MapView.tsx";
import { PinDialog } from "./components/PinDialog.tsx";
import { buildGraph, issueKey, relative } from "./graph.ts";
import type { IssueRef, Me, Overview, PinRequest } from "./protocol.ts";

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
      const ov = await api.overview();
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
    api
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
      await api.addPin(req);
      setPinOpen(false);
      setSelectedIssue(req.issue);
      await refresh();
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    }
  };

  const onUnpin = async (req: PinRequest) => {
    try {
      await api.removePin(req);
      await refresh();
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    }
  };

  const onLaunch = async (issue: IssueRef) => {
    try {
      const r = await api.launch({ issue });
      setNotice(`Launch ${r.command.id.slice(0, 8)} sent to your companion for ${issueKey(issue)} on ${r.command.branch}.`);
      await refresh();
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    }
  };

  if (needsLogin) {
    return (
      <main className="login">
        <h1>Rubato</h1>
        <p>Each developer orchestrates their own agents. Rubato keeps the whole picture.</p>
        <a className="button" href="/auth/github/login">
          Log in with GitHub
        </a>
      </main>
    );
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
