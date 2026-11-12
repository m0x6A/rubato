/**
 * From the overview to what the map draws: nodes painted by open plans, badged with issues and
 * sessions; seams badged with open plans and collisions. Pure, so it is tested without a DOM.
 */
import ELK, { type ElkNode } from "elkjs/lib/elk.bundled.js";
import type { Finding, Flow, IssueRef, IssueSummary, MapNode, Overview, PartStatus, ProgressState, Scenario, Seam, SessionState, SessionStatus } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";

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
  /** What crosses the seam, as the map declares it (ADR 0017). Empty when it declares none. */
  flows: Flow[];
  openPlans: IssueRef[];
  collision: boolean;
  issues: IssueSummary[];
  /** Findings that are about this seam: a plan's flow that the seam does not join. */
  findings: Finding[];
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
    } else if (f.kind === "flow_off_seam") {
      edgeById.get(f.seam)?.findings.push(f);
    } else if (f.kind === "unknown_ids" || f.kind === "plan_parse_error" || f.kind === "map_version_mismatch") {
      const issue = ov.issues.find((i) => issueKey(i.issue) === issueKey(f.issue));
      for (const pin of issue?.pins ?? []) {
        if (pin.kind === "node") {
          byId.get(pin.id)?.findings.push(f);
        }
      }
    }
  }
  return { nodes, edges, scenarios: ov.map.map.scenarios ?? [] };
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
