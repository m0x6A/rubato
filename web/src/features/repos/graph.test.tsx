import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ConnectedRepo, GraphOverview, GraphSourceStatus } from "../../protocol.ts";
import { GraphLine, RackUnit, SharedGraphLines, rackScale } from "./ConnectedRepos.tsx";
import { graphLine, graphOf } from "./graph.ts";

const NOW = Date.parse("2026-11-26T18:00:00Z");
const repo: ConnectedRepo = { repo: "m0x6A/rubato", in_map: true, covered: true, open_issues: 0, plans: 0, sessions: 0, unattributed: 0 };
const source = (extra: Partial<GraphSourceStatus>): GraphSourceStatus => ({
  source: "repo/m0x6A/rubato",
  kind: "repo",
  head_sha: "b35788be4eed6ce6c4656291ac1409d79c1e42f8",
  generated_at: "2026-11-26T17:00:04Z",
  checked_at: "2026-11-26T17:48:00Z",
  nodes: 5120,
  edges: 21877,
  ...extra,
});
const graph = (...sources: GraphSourceStatus[]): GraphOverview => ({ regen_secs: 900, sources });

describe("how a context graph source stands (issue #333, ADR 0084)", () => {
  it("current: its head, when a tick last found it current, and its size", () => {
    expect(graphLine(source({}), 900, NOW)).toEqual({ state: "current", text: "b35788b · checked 12m ago · 5,120 nodes" });
  });

  it("failing: the error first, and that the graph shown is the last good one", () => {
    const line = graphLine(source({ error: "tarball: 502 from the archive host" }), 900, NOW);
    expect(line.state).toBe("failing");
    expect(line.text).toBe("failing: tarball: 502 from the archive host · last good b35788b, checked 12m ago");
  });

  it("stale: no tick has found it current for three intervals", () => {
    // 45 minutes and a second at a 15-minute interval.
    const line = graphLine(source({ checked_at: "2026-11-26T17:14:59Z" }), 900, NOW);
    expect(line).toEqual({ state: "stale", text: "stale: b35788b · checked 45m ago · 5,120 nodes" });
    // Exactly three intervals is not yet stale; a longer interval moves the bar.
    expect(graphLine(source({ checked_at: "2026-11-26T17:15:00Z" }), 900, NOW).state).toBe("current");
    expect(graphLine(source({ checked_at: "2026-11-26T17:14:59Z" }), 1800, NOW).state).toBe("current");
  });

  it("not built yet, cut by a cap, and a workspace's map version instead of a head", () => {
    expect(graphLine({ source: "team", kind: "team" }, 900, NOW)).toEqual({ state: "pending", text: "not built yet" });
    expect(graphLine({ source: "team", kind: "team", error: "plans: not read" }, 900, NOW)).toEqual({ state: "failing", text: "failing: plans: not read · never built" });
    expect(graphLine(source({ truncated: true }), 900, NOW).text).toBe("b35788b · checked 12m ago · 5,120 nodes · cut by a cap");
    const ws = graphLine({ source: "workspace/default", kind: "workspace", map_version: "b35788b", generated_at: "2026-11-26T17:00:04Z", checked_at: "2026-11-26T17:59:00Z", nodes: 29 }, 900, NOW);
    expect(ws.text).toBe("map b35788b · checked 1m ago · 29 nodes");
  });

  it("finds a repository's source whatever the case of its name, and none with the flag off", () => {
    const g = graph(source({}), { source: "team", kind: "team" });
    expect(graphOf(g, "M0X6a/Rubato")?.source).toBe("repo/m0x6A/rubato");
    expect(graphOf(g, "m0x6A/other")).toBeUndefined();
    expect(graphOf(undefined, "m0x6A/rubato")).toBeUndefined();
    expect(graphOf(null, "m0x6A/rubato")).toBeUndefined();
  });
});

describe("the Repos view says whether the context graph is flowing", () => {
  it("a repository's rack unit carries its source's line, with its state as a class and in words", () => {
    render(<RackUnit repo={repo} max={rackScale([repo])} graph={source({})} regenSecs={900} now={NOW} />);
    const line = screen.getByTestId("graph-m0x6A/rubato");
    expect(line).toHaveTextContent("context graph: b35788b · checked 12m ago · 5,120 nodes");
    expect(line).toHaveAttribute("data-state", "current");
  });

  it("failing and stale are said in words, not only in colour", () => {
    render(<GraphLine name="m0x6A/rubato" source={source({ error: "head: rate limit" })} regenSecs={900} now={NOW} />);
    expect(screen.getByTestId("graph-m0x6A/rubato")).toHaveTextContent("context graph: failing: head: rate limit");
    expect(screen.getByTestId("graph-m0x6A/rubato")).toHaveAttribute("data-state", "failing");
  });

  it("with the flag off there is no line at all: the unit is as before", () => {
    render(<RackUnit repo={repo} max={rackScale([repo])} />);
    expect(screen.queryByTestId("graph-m0x6A/rubato")).toBeNull();
  });

  it("the workspace's and the team's sources show once, above the repositories", () => {
    const g = graph(source({}), { source: "workspace/default", kind: "workspace", map_version: "b35788b", generated_at: "2026-11-26T17:00:04Z", checked_at: "2026-11-26T17:59:00Z", nodes: 29 }, { source: "workspace/shop", kind: "workspace" }, { source: "team", kind: "team" });
    render(<SharedGraphLines graph={g} workspace="default" now={NOW} />);
    expect(screen.getByTestId("graph-workspace/default")).toHaveTextContent("this workspace's map and pins: map b35788b · checked 1m ago · 29 nodes");
    expect(screen.getByTestId("graph-team")).toHaveTextContent("the team's issues, plans and pull requests: not built yet");
    // Another workspace's source belongs to that workspace's view.
    expect(screen.queryByTestId("graph-workspace/shop")).toBeNull();
  });
});
