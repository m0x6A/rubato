/**
 * A plan's footprint on the system, small enough for a row of the Plans tab (issue #81, design
 * §6): every part a dot where the team put it (or in layers by its seams when the layout has not
 * placed it), the plan's affected parts lit cyan, its new parts violet, its seams lit.
 */
import type { MapLayout, Plan, SystemMap } from "../../protocol.ts";

type XY = { x: number; y: number };

/** Layers by seam direction (longest path from a part nothing points at), then order within a layer. */
export function layered(map: SystemMap): Map<string, XY> {
  const depth = new Map<string, number>(map.nodes.map((n) => [n.id, 0]));
  const seams = map.seams ?? [];
  for (let pass = 0; pass < map.nodes.length; pass++) {
    let changed = false;
    for (const s of seams) {
      const d = (depth.get(s.from) ?? 0) + 1;
      if (depth.has(s.to) && d > (depth.get(s.to) ?? 0) && d < map.nodes.length) {
        depth.set(s.to, d);
        changed = true;
      }
    }
    if (!changed) {
      break;
    }
  }
  const rows = new Map<number, number>();
  const out = new Map<string, XY>();
  for (const n of map.nodes) {
    const col = depth.get(n.id) ?? 0;
    const row = rows.get(col) ?? 0;
    rows.set(col, row + 1);
    out.set(n.id, { x: col, y: row });
  }
  return out;
}

const W = 120;
const H = 68;
const PAD = 8;

/** Where each part sits in the 120 × 68 box: the team's layout when it places every part, else layers. */
export function miniPositions(map: SystemMap, layout?: MapLayout): Map<string, XY> {
  const stored = layout?.positions ?? {};
  const all = map.nodes.every((n) => stored[n.id]);
  const raw = all ? new Map(map.nodes.map((n) => [n.id, { x: stored[n.id]!.x, y: stored[n.id]!.y }])) : layered(map);
  const xs = [...raw.values()].map((p) => p.x);
  const ys = [...raw.values()].map((p) => p.y);
  const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const sx = (W - 2 * PAD) / Math.max(1, maxX - minX);
  const sy = (H - 2 * PAD) / Math.max(1, maxY - minY);
  return new Map([...raw].map(([id, p]) => [id, { x: PAD + (p.x - minX) * sx, y: PAD + (p.y - minY) * sy }]));
}

export function MiniMap({ map, layout, plan }: { map: SystemMap; layout?: MapLayout | undefined; plan: Plan }) {
  const at = miniPositions(map, layout);
  const status = new Map(plan.affected_parts.map((p) => [p.node, p.status]));
  const litSeams = new Set(plan.affected_parts.flatMap((p) => p.seams ?? []));
  const lit = plan.affected_parts.filter((p) => p.status !== "quiet").map((p) => p.node);
  return (
    <svg className="mini-map" data-testid="mini-map" viewBox={`0 0 ${W} ${H}`} width={W} height={H} role="img" aria-label={`Where it lands: ${lit.join(", ") || "no part"}`}>
      {(map.seams ?? []).map((s) => {
        const a = at.get(s.from);
        const b = at.get(s.to);
        return a && b ? <line key={s.id} x1={a.x} y1={a.y} x2={b.x} y2={b.y} className={litSeams.has(s.id) ? "mini-seam lit" : "mini-seam"} data-seam={s.id} /> : null;
      })}
      {map.nodes.map((n) => {
        const p = at.get(n.id);
        const st = status.get(n.id);
        const cls = st === "affected" ? "mini-part affected" : st === "new" ? "mini-part new" : "mini-part";
        return p ? <circle key={n.id} cx={p.x} cy={p.y} r={st === "affected" || st === "new" ? 4.5 : 3} className={cls} data-part={n.id} /> : null;
      })}
    </svg>
  );
}
