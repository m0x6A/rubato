/**
 * What the observed system and the map disagree about, with what a person can do about each
 * (issue #140): take it into the shared map draft, which opens the form filled in from the
 * telemetry, or dismiss it for the workspace (ADR 0060). Nothing changes until a person clicks,
 * and nothing reaches the map file until a person merges the map's pull request.
 */
import type { DiscrepancyKey, Overview } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";
import { type Take, discrepancies, dismissalText } from "./territory.ts";

interface Props {
  overview: Overview;
  onTake: (take: Take) => void;
  onDismiss: (key: DiscrepancyKey) => void;
  onUndo: (id: number) => void;
  /** A write is under way: the buttons wait. */
  busy?: boolean | undefined;
  /** Who may write: without a login the buttons are not offered. */
  canWrite?: boolean | undefined;
  now?: number | undefined;
}

const takeLabel = (t: Take): string => (t.kind === "add_node" ? "Add as a part…" : t.kind === "add_seam" ? "Add as a seam…" : t.kind === "select_seam" ? "Open the seam…" : t.why);

export function Discrepancies({ overview, onTake, onDismiss, onUndo, busy = false, canWrite = true, now = Date.now() }: Props) {
  const t = overview.traffic;
  // Unknown offers nothing: silence in the data is never read as silence on the map (ADR 0039).
  if (!t || t.state !== "fresh") {
    return null;
  }
  const rows = discrepancies(overview);
  const dismissed = t.dismissed ?? [];
  return (
    <details className="discrepancies" aria-label="Map versus territory" data-testid="discrepancies">
      <summary>
        Map versus territory <span className="muted">({rows.length})</span>
      </summary>
      {rows.length === 0 ? (
        <p className="muted">The observed system matches the map.</p>
      ) : (
        <ul className="plain">
          {rows.map((r, i) => (
            <li key={i} data-testid={`discrepancy-${r.key.kind}`}>
              <span>{r.text}</span>
              {canWrite && (
                <span className="discrepancy-actions">
                  <button type="button" disabled={busy || r.take.kind === "blocked"} onClick={() => onTake(r.take)} title={r.take.kind === "blocked" ? r.take.why : "Opens the map draft with this filled in; you add it"}>
                    {takeLabel(r.take)}
                  </button>
                  <button type="button" disabled={busy} onClick={() => onDismiss(r.key)} title="Hide it for everyone in this workspace; undo below">
                    Dismiss
                  </button>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {t.ghosts_truncated && <p className="muted">More services were seen than are shown here.</p>}
      {dismissed.length > 0 && (
        <details>
          <summary>Dismissed ({dismissed.length})</summary>
          <ul className="plain">
            {dismissed.map((d) => (
              <li key={d.id} data-testid="dismissed">
                <span>
                  {dismissalText(d)} <span className="muted">· by {d.author}, {relative(d.at, now)}</span>
                </span>
                {canWrite && (
                  <button type="button" disabled={busy} onClick={() => onUndo(d.id)}>
                    Undo
                  </button>
                )}
              </li>
            ))}
          </ul>
        </details>
      )}
    </details>
  );
}
