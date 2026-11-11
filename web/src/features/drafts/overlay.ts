/**
 * The draft as an overlay on the map (ADR 0018), as pure functions: which parts and seams the
 * draft adds, changes or removes, who did it, and the map to draw while a draft exists.
 */
import type { DraftOp, MapDraft, MapNode, Overview, Seam, SystemMap } from "../../protocol.ts";
import type { MapOp } from "./api.ts";

export type DraftMarkState = "added" | "changed" | "removed";

export interface DraftMark {
  state: DraftMarkState;
  /** Who made the latest operation on it. */
  author: string;
}

/** The id an operation is about. */
export function opTarget(op: MapOp | DraftOp): string {
  return "node" in op ? op.node.id : "seam" in op ? op.seam.id : op.id;
}

/** An operation in words, as the pull request lists it. */
export function opSummary(op: MapOp | DraftOp): string {
  switch (op.op) {
    case "add_node":
      return `add part ${op.node.id}`;
    case "update_node":
      return `change part ${op.node.id}`;
    case "remove_node":
      return `remove part ${op.id}`;
    case "add_seam":
      return `add seam ${op.seam.id} (${op.seam.from} to ${op.seam.to})`;
    case "update_seam":
      return `change seam ${op.seam.id}`;
    case "remove_seam":
      return `remove seam ${op.id}`;
  }
}

/** What the draft does to each id it touches. Stale operations mark nothing: they do not apply. */
export function draftMarks(merged: SystemMap, draft: MapDraft | undefined): Map<string, DraftMark> {
  const marks = new Map<string, DraftMark>();
  const inMerged = new Set([...merged.nodes.map((n) => n.id), ...(merged.seams ?? []).map((s) => s.id)]);
  for (const o of draft?.ops ?? []) {
    if (o.stale) {
      continue;
    }
    const id = opTarget(o);
    const removes = o.op === "remove_node" || o.op === "remove_seam";
    if (removes && !inMerged.has(id)) {
      // Added and removed again inside the draft: nothing to show.
      marks.delete(id);
      continue;
    }
    marks.set(id, { state: removes ? "removed" : inMerged.has(id) ? "changed" : "added", author: o.author });
  }
  return marks;
}

/**
 * The overview to draw while a draft exists: the draft's preview, plus what the draft removes
 * (kept on the map, marked, so a removal is seen and not just missing). Pins, plans and
 * progress are untouched: they are about the merged map.
 */
export function withDraft(ov: Overview): Overview {
  const preview = ov.draft?.preview;
  if (!preview) {
    return ov;
  }
  const merged = ov.map.map;
  const nodeIds = new Set(preview.nodes.map((n) => n.id));
  const seamIds = new Set((preview.seams ?? []).map((s) => s.id));
  const nodes: MapNode[] = [...preview.nodes, ...merged.nodes.filter((n) => !nodeIds.has(n.id))];
  const allNodes = new Set(nodes.map((n) => n.id));
  const seams: Seam[] = [...(preview.seams ?? []), ...(merged.seams ?? []).filter((s) => !seamIds.has(s.id) && allNodes.has(s.from) && allNodes.has(s.to))];
  return { ...ov, map: { ...ov.map, map: { ...preview, nodes, seams } } };
}

/** The seam a drag from one part's handle to another asks for. Null when it is no seam at all. */
export function seamOpFor(from: string | null, to: string | null): MapOp | null {
  if (!from || !to || from === to) {
    return null;
  }
  // The side depended on owns the seam until someone says otherwise in the form.
  return { op: "add_seam", seam: { id: `${from}-${to}`, from, to, owner: to } };
}

/** Flows as one line each (`data: web -> server: PinRequest`), for a text area. */
export function flowsToText(flows: Seam["flows"]): string {
  return (flows ?? []).map((f) => `${f.kind}: ${f.from} -> ${f.to}: ${f.carries}`).join("\n");
}

/** The reverse of `flowsToText`. Throws with the line that does not read. */
export function flowsFromText(text: string): NonNullable<Seam["flows"]> {
  return text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .map((line) => {
      const m = /^(data|control)\s*:\s*(\S+)\s*->\s*(\S+)\s*:\s*(.+)$/.exec(line);
      if (!m) {
        throw new Error(`"${line}" is not a flow. Write: data: from -> to: what it carries`);
      }
      return { kind: m[1] as "data" | "control", from: m[2] as string, to: m[3] as string, carries: (m[4] as string).trim() };
    });
}
