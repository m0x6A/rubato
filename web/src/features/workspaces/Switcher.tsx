import type { Overview } from "../../protocol.ts";

/**
 * Which application the map shows (issue #89): one choice per workspace the server holds. Not
 * drawn when there is only one, so a single-workspace team sees no change.
 */
export function WorkspaceSwitcher({ overview, onSwitch }: { overview: Overview; onSwitch: (id: string) => void }) {
  const all = overview.workspaces ?? [];
  if (all.length < 2) {
    return null;
  }
  return (
    <label className="workspace-switch" data-testid="workspace-switch">
      <span className="muted">Workspace</span>
      <select value={overview.workspace} onChange={(e) => onSwitch(e.target.value)} aria-label="Workspace">
        {all.map((w) => (
          <option key={w.id} value={w.id}>
            {w.name}
          </option>
        ))}
      </select>
    </label>
  );
}
