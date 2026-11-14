/**
 * From the overview to what the map draws: nodes painted by open plans, badged with issues and
 * sessions; seams badged with open plans and collisions. Pure, so it is tested without a DOM.
 */
import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import type { Finding, Flow, IssueRef, IssueSummary, MapNode, ObservedTraffic, Overview, PartStatus, ProgressState, Scenario, Seam, SeamTraffic, SessionState, SessionStatus, UndeclaredEdge } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";

export type Paint = PartStatus | null;

export interface GraphNode {
  id: string;
  kind: MapNode["kind"];
  description: string | undefined;
  /** `owner/name` of the repo holding its code, if the map says (issue #89: the By repo grouping). */
  repo: string | undefined;
  issues: IssueSummary[];
  sessions: SessionState[];
  paint: Paint;
  findings: Finding[];
  /** Issues with no pin whose open plan names this part (issue #80): planned here. */
  planned: IssueSummary[];
  /** Data models open plans introduce or change on this part (ADR 0031). */
  models: number;
}

export interface GraphEdge {
  id: string;
  from: string;
  to: string;
  owner: string;
  carries: string | undefined;
  /** What crosses the seam, as the map declares it (ADR 0017). Empty when it declares none. */
  flows: Flow[];
  openPlans: IssueRef[];
  collision: boolean;
  issues: IssueSummary[];
  /** Findings that are about this seam: a plan's flow that the seam does not join. */
  findings: Finding[];
  /** Observed calls in the report's window, from a fresh report only (ADR 0039). */
  traffic?: SeamTraffic | undefined;
  /** Finding `SeamQuiet`: no calls in this many seconds while both parts show traffic. Amber, not a plan's finding. */
  quietFor?: number | undefined;
}

/** What the shared map draft does to a part or a seam, and who did it (ADR 0018). */
export interface DraftMarkLike {
  state: "added" | "changed" | "removed";
  author: string;
}

export interface Graph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** Journeys over the declared flows, for the scenario lens (ADR 0024). */
  scenarios: Scenario[];
  /** What the traffic source says, as the server sent it (ADR 0039). */
  traffic?: ObservedTraffic | undefined;
  /** Calls between two parts of the map that no seam joins: drawn in the Traffic lens only, from a fresh report. */
  territory?: UndeclaredEdge[] | undefined;
}

const PAINT_RANK: Record<PartStatus, number> = { quiet: 1, affected: 2, new: 3 };

function strongerPaint(a: Paint, b: PartStatus): Paint {
  if (a === null) {
    return b;
  }
  return PAINT_RANK[b] > PAINT_RANK[a] ? b : a;
}

export function isOpen(state: ProgressState): boolean {
  return state !== "done";
}

export function buildGraph(ov: Overview): Graph {
  const seams: Seam[] = ov.map.map.seams ?? [];
  const nodes: GraphNode[] = ov.map.map.nodes.map((n) => ({
    id: n.id,
    kind: n.kind,
    description: n.description ?? undefined,
    repo: n.repo ?? undefined,
    issues: ov.issues.filter((i) => i.pins.some((p) => p.kind === "node" && p.id === n.id)),
    sessions: [],
    paint: null,
    findings: [],
    planned: [],
    models: 0,
  }));
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const edges: GraphEdge[] = seams.map((s) => ({
    id: s.id,
    from: s.from,
    to: s.to,
    owner: s.owner,
    carries: s.carries ?? undefined,
    flows: s.flows ?? [],
    openPlans: [],
    collision: false,
    issues: ov.issues.filter((i) => i.pins.some((p) => p.kind === "seam" && p.id === s.id)),
    findings: [],
  }));
  const edgeById = new Map(edges.map((e) => [e.id, e]));

  for (const issue of ov.issues) {
    // Sessions land on the nodes the issue is pinned to.
    for (const pin of issue.pins) {
      if (pin.kind === "node") {
        byId.get(pin.id)?.sessions.push(...(issue.sessions ?? []));
      }
    }
    const plan = issue.plan;
    if (!plan || !isOpen(issue.progress.state)) {
      continue;
    }
    for (const m of plan.data_models ?? []) {
      const node = byId.get(m.part);
      if (node) {
        node.models += 1;
      }
    }
    for (const part of plan.affected_parts) {
      const node = byId.get(part.node);
      if (node) {
        node.paint = strongerPaint(node.paint, part.status);
        // Not pinned anywhere: the plan says where it belongs, and its sessions show there.
        if (issue.pins.length === 0 && part.status !== "quiet") {
          node.planned.push(issue);
          node.sessions.push(...(issue.sessions ?? []));
        }
      }
      for (const seam of part.seams ?? []) {
        const e = edgeById.get(seam);
        if (e && !e.openPlans.some((p) => issueKey(p) === issueKey(issue.issue))) {
          e.openPlans.push(issue.issue);
        }
      }
    }
    for (const row of plan.o_rows ?? []) {
      const e = edgeById.get(row.seam);
      if (e && !e.openPlans.some((p) => issueKey(p) === issueKey(issue.issue))) {
        e.openPlans.push(issue.issue);
      }
    }
  }
  // Unknown is never quiet: only a fresh report puts calls, quiet seams or territory on the map.
  const traffic: ObservedTraffic = ov.traffic ?? { state: "not_configured" };
  const fresh = traffic.state === "fresh";
  if (fresh) {
    for (const t of traffic.seams ?? []) {
      const e = edgeById.get(t.seam);
      if (e) {
        e.traffic = t;
      }
    }
  }
  for (const f of ov.findings) {
    if (f.kind === "seam_quiet") {
      const e = fresh ? edgeById.get(f.seam) : undefined;
      if (e) {
        e.quietFor = f.window_secs;
      }
    } else if (f.kind === "seam_collision") {
      const e = edgeById.get(f.seam);
      if (e) {
        e.collision = true;
      }
    } else if (f.kind === "flow_off_seam") {
      edgeById.get(f.seam)?.findings.push(f);
    } else if (f.kind === "unknown_ids" || f.kind === "plan_parse_error" || f.kind === "map_version_mismatch" || f.kind === "approval_stale") {
      const issue = ov.issues.find((i) => issueKey(i.issue) === issueKey(f.issue));
      for (const pin of issue?.pins ?? []) {
        if (pin.kind === "node") {
          byId.get(pin.id)?.findings.push(f);
        }
      }
    }
  }
  const territory = fresh ? (traffic.undeclared ?? []).filter((u) => byId.has(u.from) && byId.has(u.to)) : [];
  return { nodes, edges, scenarios: ov.map.map.scenarios ?? [], traffic, territory };
}

export function countBy<T, K extends string>(items: T[], key: (t: T) => K): Partial<Record<K, number>> {
  const out: Partial<Record<K, number>> = {};
  for (const it of items) {
    const k = key(it);
    out[k] = (out[k] ?? 0) + 1;
  }
  return out;
}

export function sessionCounts(sessions: SessionState[]): Partial<Record<SessionStatus, number>> {
  return countBy(sessions, (s) => s.status);
}

export function issueCounts(issues: IssueSummary[]): Partial<Record<ProgressState, number>> {
  return countBy(issues, (i) => i.progress.state);
}

/** How many of a part's issues have a plan approved as it is now, and how many an approval of
 * an earlier version (ADR 0027). */
export function approvalCounts(issues: IssueSummary[]): { approved: number; stale: number } {
  return {
    approved: issues.filter((i) => i.approval?.version === "current").length,
    stale: issues.filter((i) => i.approval?.version === "earlier").length,
  };
}

export const NODE_WIDTH = 220;
export const NODE_HEIGHT = 96;

/** ELK layered layout, left to right. Returns positions by node id. */
export async function layout(graph: Graph): Promise<Map<string, { x: number; y: number }>> {
  const elk = new ELK();
  const root: ElkNode = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.spacing.nodeNode": "40",
      // Room between layers for a seam's chip (its id, plus plan and issue counts).
      "elk.layered.spacing.nodeNodeBetweenLayers": "200",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
    },
    children: graph.nodes.map((n) => ({ id: n.id, width: NODE_WIDTH, height: NODE_HEIGHT })),
    edges: graph.edges.map((e) => ({ id: e.id, sources: [e.from], targets: [e.to] })),
  };
  const out = await elk.layout(root);
  const pos = new Map<string, { x: number; y: number }>();
  for (const c of out.children ?? []) {
    pos.set(c.id, { x: c.x ?? 0, y: c.y ?? 0 });
  }
  return pos;
}

/** A repo's box in the By repo grouping, at its place on the canvas. */
export interface RepoGroup {
  id: string;
  repo: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The node id of a repo's box; never a map id, since map ids hold no colon. */
export function repoGroupId(repo: string): string {
  return `repo:${repo}`;
}

/**
 * ELK with one compound node per repo (issue #89): each repo's parts are laid out inside its box,
 * parts with no repo (externals) beside the boxes. A part in a box gets a position relative to
 * the box and its id as `parent`, which is how React Flow draws a child (`parentId`).
 */
export async function layoutByRepo(graph: Graph): Promise<{ groups: RepoGroup[]; parts: Map<string, { x: number; y: number; parent?: string }> }> {
  const elk = new ELK();
  const repos = [...new Set(graph.nodes.map((n) => n.repo).filter((r): r is string => Boolean(r)))];
  const leaf = (id: string): ElkNode => ({ id, width: NODE_WIDTH, height: NODE_HEIGHT });
  const root: ElkNode = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": "RIGHT",
      "elk.hierarchyHandling": "INCLUDE_CHILDREN",
      "elk.spacing.nodeNode": "40",
      "elk.layered.spacing.nodeNodeBetweenLayers": "200",
      "elk.layered.nodePlacement.strategy": "NETWORK_SIMPLEX",
    },
    children: [
      ...repos.map((repo) => ({
        id: repoGroupId(repo),
        // Room at the top for the repo's name.
        layoutOptions: { "elk.padding": "[top=48,left=24,bottom=24,right=24]" },
        children: graph.nodes.filter((n) => n.repo === repo).map((n) => leaf(n.id)),
      })),
      ...graph.nodes.filter((n) => !n.repo).map((n) => leaf(n.id)),
    ],
    edges: graph.edges.map((e) => ({ id: e.id, sources: [e.from], targets: [e.to] })),
  };
  const out = await elk.layout(root);
  const groups: RepoGroup[] = [];
  const parts = new Map<string, { x: number; y: number; parent?: string }>();
  for (const c of out.children ?? []) {
    if (c.id.startsWith("repo:")) {
      groups.push({ id: c.id, repo: c.id.slice("repo:".length), x: c.x ?? 0, y: c.y ?? 0, width: c.width ?? 0, height: c.height ?? 0 });
      for (const k of c.children ?? []) {
        parts.set(k.id, { x: k.x ?? 0, y: k.y ?? 0, parent: c.id });
      }
    } else {
      parts.set(c.id, { x: c.x ?? 0, y: c.y ?? 0 });
    }
  }
  return { groups, parts };
}
