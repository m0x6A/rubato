import type { IssueSummary, Overview, Seam, SessionState } from "../../src/protocol.ts";

/**
 * A team's map larger than Rubato's own, for the performance e2e (issue #81, design §5): `parts`
 * parts in a grid of chains, `seams` seams with one data flow each, and a working session on
 * every fourth part. Typed as the generated `Overview`, so typecheck catches drift.
 */
export function bigMap(parts = 40, seams = 60, traffic = false): Overview {
  const now = new Date().toISOString();
  const ids = Array.from({ length: parts }, (_, i) => `part-${String(i).padStart(2, "0")}`);
  const edges: Seam[] = [];
  for (let i = 0; edges.length < seams; i++) {
    const from = ids[i % parts] as string;
    const to = ids[(i + 1 + Math.floor(i / parts) * 3) % parts] as string;
    if (from === to || edges.some((e) => e.from === from && e.to === to)) {
      continue;
    }
    edges.push({ id: `${from}--${to}`, from, to, owner: to, flows: [{ kind: "data", from: to, to: from, carries: `rows of ${to}` }] });
  }
  const issues: IssueSummary[] = ids
    .filter((_, i) => i % 4 === 0)
    .map((id, i) => {
      const issue = { repo: "team/big", number: 100 + i };
      const session: SessionState = { companion_id: "c1", user: "m0x6A", cwd: `/w/${issue.number}`, issue, source: "hook", status: "working", last_seen: now };
      return { issue, pins: [{ kind: "node", id }], progress: { issue, state: "building", last_seen: now }, sessions: [session] };
    });
  return {
    map: { version: "big0000", loaded_at: now, map: { name: "big", version: 1, nodes: ids.map((id) => ({ id, kind: "context", description: `the ${id} context` })), seams: edges } },
    issues,
    findings: [],
    companions: [],
    unpinned_sessions: [],
    generated_at: now,
    // Issue #104: 60 calls a minute each way on every seam, 6 chevrons a seam, so the cap of 120 bites.
    ...(traffic ? { traffic: { state: "fresh" as const, observed_at: now, window_secs: 3600, seams: edges.map((e) => ({ seam: e.id, forward_calls: 3600, reverse_calls: 3600, errors: 0 })), observed_parts: ids } } : {}),
  };
}
