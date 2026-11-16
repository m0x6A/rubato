import type { AgentLine, SessionState } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";
import { SESSION_STALE_SECS } from "./Lanes.tsx";

/**
 * The newest line a launched session's agent said it is doing (ADR 0071, plan 226 §3), joined on
 * `(companion_id, launch_id)`: a launch id another companion claims is not this session's line.
 */
export function agentLineFor(lines: AgentLine[] | undefined, s: SessionState): AgentLine | undefined {
  if (!s.launch_id) {
    return undefined;
  }
  return (lines ?? []).find((l) => l.launch_id === s.launch_id && l.companion_id === s.companion_id);
}

/**
 * The line in quotes with its age and the parts it names, as text. A signal, never a state: no
 * chip, no colour, and never a link. Muted when it is older than the session's own last signal
 * by more than the stale window: still shown, its age says how much to trust it.
 */
export function AgentLineView({ line, session }: { line: AgentLine; session: SessionState }) {
  const old = Date.parse(session.last_seen) - Date.parse(line.reported_at) > SESSION_STALE_SECS * 1000;
  return (
    <div className={old ? "agent-line muted" : "agent-line"} data-testid="agent-line" title="What the agent says it is doing: a signal, not a progress state">
      "{line.doing}" · {relative(line.reported_at)}
      {(line.parts ?? []).length > 0 && <span className="agent-parts"> · {(line.parts ?? []).join(", ")}</span>}
    </div>
  );
}
