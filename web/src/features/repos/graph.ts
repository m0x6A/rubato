import type { GraphOverview, GraphSourceStatus } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";

/** How a context graph source stands, as the Repos view says it (ADR 0084). */
export type GraphState = "current" | "stale" | "failing" | "pending";

/** A source is stale when no tick has found it current for this many intervals, as the alert
 * `ContextGraphStale` judges it. */
export const STALE_AFTER_INTERVALS = 3;

/** The source of repository `repo` (`owner/name`), if the server builds one. */
export function graphOf(graph: GraphOverview | null | undefined, repo: string): GraphSourceStatus | undefined {
  const key = `repo/${repo}`.toLowerCase();
  return (graph?.sources ?? []).find((s) => s.source.toLowerCase() === key);
}

/** What one source is at: a repository's head, or a workspace's map version. */
function at(s: GraphSourceStatus): string | undefined {
  if (s.head_sha) {
    return s.head_sha.slice(0, 7);
  }
  return s.map_version ? `map ${s.map_version.slice(0, 7)}` : undefined;
}

/** One line for a source: its state, and the words that say it. Every signal carries its
 * last-seen time: the age is from `checked_at`, when a tick last found the rows current. */
export function graphLine(s: GraphSourceStatus, regenSecs: number, now = Date.now()): { state: GraphState; text: string } {
  const checked = s.checked_at ? `checked ${relative(s.checked_at, now)}` : undefined;
  if (s.error) {
    const last = s.generated_at ? `last good ${[at(s), checked].filter(Boolean).join(", ")}` : "never built";
    return { state: "failing", text: `failing: ${s.error} · ${last}` };
  }
  if (!s.generated_at) {
    return { state: "pending", text: "not built yet" };
  }
  const parts = [at(s), checked, `${(s.nodes ?? 0).toLocaleString("en-US")} nodes`, s.truncated ? "cut by a cap" : undefined].filter(Boolean);
  const age = s.checked_at ? (now - Date.parse(s.checked_at)) / 1000 : Number.POSITIVE_INFINITY;
  if (age > STALE_AFTER_INTERVALS * regenSecs) {
    return { state: "stale", text: `stale: ${parts.join(" · ")}` };
  }
  return { state: "current", text: parts.join(" · ") };
}
