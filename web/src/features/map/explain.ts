/**
 * What the map can say about one part or one seam (design S26): what it is, what goes in and
 * out, what depends on what, and what touches it now. Pure functions over the overview, so the
 * explanation is tested without a DOM.
 *
 * Direction: a seam runs from the side that depends to the side depended on.
 */
import type { DecisionRecord, Flow, IssueRef, IssueSummary, MapNode, MapRef, ORow, Overview, Seam, SessionState } from "../../protocol.ts";

export interface FlowLine {
  flow: Flow;
  /** The part at the other end. */
  other: string;
  seam: string;
}

export interface PlanTouch {
  issue: IssueRef;
  title: string | undefined;
  /** What the plan says it changes here, for a part. */
  change: string | undefined;
  status: string | undefined;
}

export interface Touching {
  /** Issues pinned to exactly this part or seam. */
  issues: IssueSummary[];
  /** Open plans that name it. */
  plans: PlanTouch[];
  sessions: SessionState[];
  decisions: DecisionRecord[];
}

export interface PartExplanation {
  kind: "node";
  node: MapNode;
  /** A link to the code, when the map gives a repo. */
  codeUrl: string | undefined;
  incoming: FlowLine[];
  outgoing: FlowLine[];
  dependsOn: { part: string; seam: string }[];
  dependants: { part: string; seam: string }[];
  /** Seams of this part that declare no flows: a gap in what the team has written down. */
  seamsWithoutFlows: string[];
  touching: Touching;
}

export interface SeamExplanation {
  kind: "seam";
  seam: Seam;
  flows: Flow[];
  /** O-rows open plans have written for this seam. */
  oRows: { issue: IssueRef; row: ORow }[];
  touching: Touching;
}

export type Explanation = PartExplanation | SeamExplanation;

const isOpen = (i: IssueSummary) => i.progress.state !== "done";
const pinnedTo = (i: IssueSummary, ref: MapRef) => i.pins.some((p) => p.kind === ref.kind && p.id === ref.id);

function touching(ov: Overview, ref: MapRef): Touching {
  const issues = ov.issues.filter((i) => pinnedTo(i, ref));
  const plans: PlanTouch[] = [];
  for (const i of ov.issues) {
    if (!i.plan || !isOpen(i)) {
      continue;
    }
    const part = ref.kind === "node" ? i.plan.affected_parts.find((p) => p.node === ref.id) : undefined;
    const names = ref.kind === "node" ? Boolean(part) : i.plan.affected_parts.some((p) => (p.seams ?? []).includes(ref.id)) || (i.plan.o_rows ?? []).some((r) => r.seam === ref.id);
    if (names) {
      plans.push({ issue: i.issue, title: i.title ?? i.plan.title ?? undefined, change: part?.change, status: part?.status });
    }
  }
  // Sessions and decisions belong to the issues pinned here.
  return { issues, plans, sessions: issues.flatMap((i) => i.sessions ?? []), decisions: issues.flatMap((i) => i.decisions ?? []) };
}

export function explain(ov: Overview, ref: MapRef): Explanation | null {
  const map = ov.map.map;
  const seams = map.seams ?? [];
  if (ref.kind === "seam") {
    const seam = seams.find((s) => s.id === ref.id);
    if (!seam) {
      return null;
    }
    const oRows = ov.issues.filter((i) => i.plan && isOpen(i)).flatMap((i) => (i.plan?.o_rows ?? []).filter((r) => r.seam === seam.id).map((row) => ({ issue: i.issue, row })));
    return { kind: "seam", seam, flows: seam.flows ?? [], oRows, touching: touching(ov, ref) };
  }
  const node = map.nodes.find((n) => n.id === ref.id);
  if (!node) {
    return null;
  }
  const mine = seams.filter((s) => s.from === node.id || s.to === node.id);
  const lines = (pick: (f: Flow) => boolean, other: (f: Flow) => string): FlowLine[] => mine.flatMap((s) => (s.flows ?? []).filter(pick).map((flow) => ({ flow, other: other(flow), seam: s.id })));
  return {
    kind: "node",
    node,
    codeUrl: node.repo ? `https://github.com/${node.repo}${node.path ? `/tree/HEAD/${node.path}` : ""}` : undefined,
    incoming: lines(
      (f) => f.to === node.id,
      (f) => f.from,
    ),
    outgoing: lines(
      (f) => f.from === node.id,
      (f) => f.to,
    ),
    dependsOn: mine.filter((s) => s.from === node.id).map((s) => ({ part: s.to, seam: s.id })),
    dependants: mine.filter((s) => s.to === node.id).map((s) => ({ part: s.from, seam: s.id })),
    seamsWithoutFlows: mine.filter((s) => !s.flows?.length).map((s) => s.id),
    touching: touching(ov, ref),
  };
}

/**
 * What stays lit while something is selected: a part with its neighbours and the seams between
 * them, or a seam with its two ends. Null when nothing is selected: nothing dims.
 */
export function lit(seams: readonly { id: string; from: string; to: string }[], selection: MapRef | null): { nodes: Set<string>; seams: Set<string> } | null {
  if (!selection) {
    return null;
  }
  if (selection.kind === "seam") {
    const s = seams.find((x) => x.id === selection.id);
    return s ? { nodes: new Set([s.from, s.to]), seams: new Set([s.id]) } : null;
  }
  const mine = seams.filter((s) => s.from === selection.id || s.to === selection.id);
  return { nodes: new Set([selection.id, ...mine.flatMap((s) => [s.from, s.to])]), seams: new Set(mine.map((s) => s.id)) };
}
