/**
 * The observed system over the declared map (issue #140, ADRs 0059, 0060), as pure functions:
 * where ghost parts sit, what each discrepancy is called, and what "Take into draft" fills in.
 * A ghost is a service seen in a fresh report that no part names. It is never a map part: it has
 * no id the map knows, takes no drop and never reaches the shared layout.
 */
import type { DiscrepancyKey, DraftOp, GhostEdge, GhostPart, NodeKind, ObservedTraffic, Overview, ServiceId, TrafficDismissal, TrafficEnd } from "../../protocol.ts";
import { type XY } from "./layout.ts";
import { NODE_HEIGHT, NODE_WIDTH } from "./graph.ts";

/** A ghost's node id on the canvas. Map ids hold no colon, so it never collides with a part. */
export function ghostId(s: ServiceId): string {
  return `ghost:${s.namespace ?? ""}/${s.name}`;
}

export function isGhostId(id: string): boolean {
  return id.startsWith("ghost:");
}

/** `namespace/name`, or `name` alone when the service emits no namespace. */
export function serviceText(s: ServiceId): string {
  return s.namespace ? `${s.namespace}/${s.name}` : s.name;
}

export function sameService(a: ServiceId | null | undefined, b: ServiceId | null | undefined): boolean {
  return !!a && !!b && a.name === b.name && (a.namespace ?? null) === (b.namespace ?? null);
}

/** The canvas id of an edge end: the part's id, or its ghost's. */
export function endId(e: TrafficEnd): string {
  return e.kind === "part" ? e.id : ghostId(e.service);
}

export function endText(e: TrafficEnd): string {
  return e.kind === "part" ? e.id : serviceText(e.service);
}

/** Room between the declared map's right edge and the ghost band. */
export const BAND_GAP = 200;

/**
 * Where the ghosts sit (plan #140, Q5): one column to the right of the declared map, sorted by
 * namespace then name, from its top. The declared positions are read, never changed.
 */
export function ghostBand(declared: ReadonlyMap<string, XY>, ghosts: readonly GhostPart[]): Map<string, XY> {
  const xs = [...declared.values()];
  const right = xs.length ? Math.max(...xs.map((p) => p.x)) + NODE_WIDTH : 0;
  const top = xs.length ? Math.min(...xs.map((p) => p.y)) : 0;
  const sorted = [...ghosts].sort((a, b) => (a.service.namespace ?? "").localeCompare(b.service.namespace ?? "") || a.service.name.localeCompare(b.service.name));
  return new Map(sorted.map((g, i) => [ghostId(g.service), { x: right + BAND_GAP, y: top + i * (NODE_HEIGHT + 40) }]));
}

/** The ghosts and their edges to draw: a fresh report only, every end on the canvas. */
export function ghostsToDraw(t: ObservedTraffic | undefined, partIds: ReadonlySet<string>): { ghosts: GhostPart[]; edges: GhostEdge[] } {
  if (!t || t.state !== "fresh") {
    return { ghosts: [], edges: [] };
  }
  const ghosts = t.ghosts ?? [];
  const shown = new Set(ghosts.map((g) => ghostId(g.service)));
  const on = (e: TrafficEnd) => (e.kind === "part" ? partIds.has(e.id) : shown.has(ghostId(e.service)));
  return { ghosts, edges: (t.ghost_edges ?? []).filter((e) => on(e.from) && on(e.to)) };
}

/** A part id a person may keep or change: the service's name made an id, prefixed with its namespace when taken. */
export function suggestedId(s: ServiceId, taken: ReadonlySet<string>): string {
  const slug = (x: string) =>
    x
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64) || "part";
  const plain = slug(s.name);
  if (!taken.has(plain) || !s.namespace) {
    return plain;
  }
  return slug(`${s.namespace}-${s.name}`);
}

/** What "Take into draft" does for one discrepancy, or why it cannot yet. */
export type Take =
  | { kind: "add_node"; id: string; nodeKind: NodeKind; service: ServiceId }
  | { kind: "add_seam"; from: string; to: string; id: string }
  | { kind: "select_seam"; seam: string }
  | { kind: "blocked"; why: string };

export interface Discrepancy {
  key: DiscrepancyKey;
  /** One line, in the person's terms. */
  text: string;
  take: Take;
}

/** The part a draft op adds for this service, if the draft has one (the edge can then follow). */
function draftedPart(ops: readonly DraftOp[], s: ServiceId): string | null {
  for (const o of ops) {
    if (o.op === "add_node" && !o.stale && sameService(o.node.service, s)) {
      return o.node.id;
    }
  }
  return null;
}

/**
 * Every discrepancy the Traffic lens offers, busiest first within each kind: ghosts, edges no
 * seam declares (to ghosts and between parts), quiet seams. Nothing unless the report is fresh:
 * unknown offers no action (ADR 0039).
 */
export function discrepancies(ov: Overview): Discrepancy[] {
  const t = ov.traffic;
  if (!t || t.state !== "fresh") {
    return [];
  }
  const ops = ov.draft?.ops ?? [];
  const map = ov.map.map;
  const taken = new Set([...map.nodes.map((n) => n.id), ...ops.flatMap((o) => (o.op === "add_node" ? [o.node.id] : []))]);
  const out: Discrepancy[] = [];
  for (const g of t.ghosts ?? []) {
    const inDraft = draftedPart(ops, g.service);
    out.push({
      key: { kind: "ghost", service: g.service },
      text: `${serviceText(g.service)} is running but not on the map (${g.calls_in + g.calls_out} calls)`,
      take: inDraft ? { kind: "blocked", why: `in the draft as ${inDraft}` } : { kind: "add_node", id: suggestedId(g.service, taken), nodeKind: "context", service: g.service },
    });
  }
  const partFor = (e: TrafficEnd): string | null => (e.kind === "part" ? e.id : draftedPart(ops, e.service));
  for (const e of t.ghost_edges ?? []) {
    const from = partFor(e.from);
    const to = partFor(e.to);
    out.push({
      key: { kind: "edge", from: e.from, to: e.to },
      text: `${endText(e.from)} calls ${endText(e.to)}, which no seam declares (${e.calls} calls)`,
      take: from && to ? { kind: "add_seam", from, to, id: `${from}-${to}` } : { kind: "blocked", why: "take the part into the draft first" },
    });
  }
  for (const u of t.undeclared ?? []) {
    out.push({
      key: { kind: "edge", from: { kind: "part", id: u.from }, to: { kind: "part", id: u.to } },
      text: `${u.from} calls ${u.to}, which no seam declares (${u.calls} calls)`,
      take: { kind: "add_seam", from: u.from, to: u.to, id: `${u.from}-${u.to}` },
    });
  }
  for (const f of ov.findings) {
    if (f.kind === "seam_quiet") {
      out.push({
        key: { kind: "quiet_seam", seam: f.seam },
        text: `${f.seam} is declared but carried no calls`,
        take: { kind: "select_seam", seam: f.seam },
      });
    }
  }
  return out;
}

/** A dismissal in words, for the legend's Dismissed list. */
export function dismissalText(d: TrafficDismissal): string {
  const k = d.target;
  switch (k.kind) {
    case "ghost":
      return `${serviceText(k.service)} not on the map`;
    case "edge":
      return `${endText(k.from)} calls ${endText(k.to)}`;
    case "quiet_seam":
      return `${k.seam} quiet`;
  }
}
