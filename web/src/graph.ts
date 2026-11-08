/**
 * From the overview to what the map draws: nodes painted by open plans, badged with issues and
 * sessions; seams badged with open plans and collisions. Pure, so it is tested without a DOM.
 */
import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import type { Finding, IssueRef, IssueSummary, MapNode, Overview, PartStatus, ProgressState, Seam, SessionState, SessionStatus } from "./protocol.ts";

export type Paint = PartStatus | null;

export interface GraphNode {
  id: string;
  kind: MapNode["kind"];
  description: string | undefined;
  issues: IssueSummary[];
  sessions: SessionState[];
  paint: Paint;
  findings: Finding[];
}

export interface GraphEdge {
  id: string;
  from: string;
  to: string;
  owner: string;
  carries: string | undefined;
  openPlans: IssueRef[];
  collision: boolean;
  issues: IssueSummary[];
}

export interface Graph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export function issueKey(i: IssueRef): string {
  return `${i.repo}#${i.number}`;
}

export function parseIssueKey(s: string): IssueRef | null {
  const m = /^([^\s/#]+\/[^\s/#]+)#(\d+)$/.exec(s.trim());
  return m ? { repo: m[1] as string, number: Number(m[2]) } : null;
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
    issues: ov.issues.filter((i) => i.pins.some((p) => p.kind === "node" && p.id === n.id)),
    sessions: [],
    paint: null,
    findings: [],
  }));
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const edges: GraphEdge[] = seams.map((s) => ({
    id: s.id,
    from: s.from,
    to: s.to,
    owner: s.owner,
    carries: s.carries ?? undefined,
    openPlans: [],
    collision: false,
    issues: ov.issues.filter((i) => i.pins.some((p) => p.kind === "seam" && p.id === s.id)),
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
    for (const part of plan.affected_parts) {
      const node = byId.get(part.node);
      if (node) {
        node.paint = strongerPaint(node.paint, part.status);
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
  for (const f of ov.findings) {
    if (f.kind === "seam_collision") {
      const e = edgeById.get(f.seam);
      if (e) {
        e.collision = true;
      }
    } else if (f.kind === "unknown_ids" || f.kind === "plan_parse_error" || f.kind === "map_version_mismatch") {
      const issue = ov.issues.find((i) => issueKey(i.issue) === issueKey(f.issue));
      for (const pin of issue?.pins ?? []) {
        if (pin.kind === "node") {
          byId.get(pin.id)?.findings.push(f);
        }
      }
    }
  }
  return { nodes, edges };
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
      "elk.layered.spacing.nodeNodeBetweenLayers": "90",
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

export const STATE_LABEL: Record<ProgressState, string> = {
  planned: "Planned",
  started: "Started",
  designing: "Designing",
  designed: "Designed",
  building: "Building",
  blocked: "Blocked",
  in_review: "In review",
  done: "Done",
  unknown: "Unknown",
};

export function findingText(f: Finding): string {
  switch (f.kind) {
    case "unknown_ids":
      return `${issueKey(f.issue)} names ids the map does not have: ${f.ids.join(", ")}`;
    case "seam_collision":
      return `two open plans touch ${f.seam}: ${f.issues.map(issueKey).join(", ")}`;
    case "plan_parse_error":
      return `${issueKey(f.issue)}: ${f.path} does not parse: ${f.error}`;
    case "map_version_mismatch":
      return `${issueKey(f.issue)} was launched with map ${f.launched_with}; the server holds ${f.current}`;
  }
}

export function relative(iso: string, now = Date.now()): string {
  const secs = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (secs < 60) {
    return `${secs}s ago`;
  }
  if (secs < 3600) {
    return `${Math.round(secs / 60)}m ago`;
  }
  if (secs < 86400) {
    return `${Math.round(secs / 3600)}h ago`;
  }
  return `${Math.round(secs / 86400)}d ago`;
}
