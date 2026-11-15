/**
 * Seams inside one repo, folded into one bundle per pair of parts in the By repo grouping (issue
 * #144, decision 3). What someone opening By repo needs first is what crosses from one repo to
 * another; a seam inside a repo is one line with a small badge until it is opened. A warning on
 * any of its seams stays on the badge. Pure, so it is tested without a DOM.
 */
import type { GraphEdge } from "./graph.ts";

export interface Bundle {
  /** The unordered pair, `a|b` with `a < b`. */
  key: string;
  /** The ends of its line: the first seam's `from` and `to`. */
  from: string;
  to: string;
  seams: GraphEdge[];
}

export function bundleKey(a: string, b: string): string {
  return a < b ? `${a}|${b}` : `${b}|${a}`;
}

/** Seams whose two parts name the same repo, by pair. A seam to a part with no repo, or across repos, is never bundled. */
export function bundlesOf(nodes: readonly { id: string; repo: string | undefined }[], edges: readonly GraphEdge[]): Map<string, Bundle> {
  const repoOf = new Map(nodes.map((n) => [n.id, n.repo]));
  const out = new Map<string, Bundle>();
  for (const e of edges) {
    const a = repoOf.get(e.from);
    if (!a || a !== repoOf.get(e.to)) {
      continue;
    }
    const key = bundleKey(e.from, e.to);
    const b = out.get(key);
    if (b) {
      b.seams.push(e);
    } else {
      out.set(key, { key, from: e.from, to: e.to, seams: [e] });
    }
  }
  return out;
}

export interface OpenBy {
  /** Seams lit by the selection, the plan lens or the scenario step; null when nothing is. */
  lit: ReadonlySet<string> | null;
  /** Bundles under the pointer, or under an issue being dragged. */
  hovered: ReadonlySet<string>;
  /** Bundles opened with a click on their badge. */
  pinned: ReadonlySet<string>;
}

export function isOpen(b: Bundle, by: OpenBy): boolean {
  return by.hovered.has(b.key) || by.pinned.has(b.key) || (by.lit !== null && b.seams.some((s) => by.lit!.has(s.id)));
}

export interface BadgeState {
  /** The chip class of the strongest state among its seams, as `SeamChip` would draw it. */
  tone: "collision" | "quiet" | "busy" | "";
  /** A collision or finding: shown as ⚠ so it never folds away. */
  warn: boolean;
  /** Some seam in it is added, changed or removed in the shared draft. */
  draft: boolean;
  /** What the badge says. */
  text: string;
  /** Its title: every seam in it. */
  title: string;
}

export function badgeState(b: Bundle, inDraft: (id: string) => boolean): BadgeState {
  const warn = b.seams.some((s) => s.collision || s.findings.length > 0);
  const tone = warn ? "collision" : b.seams.some((s) => s.quietFor !== undefined) ? "quiet" : b.seams.some((s) => s.openPlans.length > 0) ? "busy" : "";
  const n = b.seams.length;
  return {
    tone,
    warn,
    draft: b.seams.some((s) => inDraft(s.id)),
    text: `${n} seam${n === 1 ? "" : "s"}`,
    title: `${b.seams.map((s) => s.id).join(", ")}: inside one repo. Point at it, click it or select a part to show ${n === 1 ? "it" : "them"}.`,
  };
}
