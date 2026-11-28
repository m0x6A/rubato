import { useState } from "react";
import type { Overview } from "../../protocol.ts";
import { Confirm } from "../../shared/Confirm.tsx";
import { DEFAULT_WORKSPACE } from "../../shared/workspace.ts";
import { NewWorkspaceForm } from "./NewWorkspaceForm.tsx";
import { refusalText, settingsApi } from "./api.ts";

interface Props {
  overview: Overview;
  onClose: () => void;
  /** A workspace was deleted: the list the overview carries is stale. */
  onChanged: () => void;
  onNotice: (text: string) => void;
  onSwitch: (id: string) => void;
}

/**
 * Workspaces (issue #189, ADR 0075; its own page since issue #357): the applications this tenant
 * holds, a new one, and deleting one. What one workspace reads and how is Configure workspace.
 */
export function WorkspacesView({ overview, onClose, onChanged, onNotice, onSwitch }: Props) {
  const current = overview.workspace ?? DEFAULT_WORKSPACE;
  // A server with one workspace lists none: the one shown is then the whole list.
  const listed = overview.workspaces ?? [];
  const all = listed.length > 0 ? listed : [{ id: current, name: overview.map.map.name }];
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const remove = async (id: string, name: string) => {
    setBusy(true);
    setMessage(null);
    try {
      await settingsApi.remove(id);
      onNotice(`Workspace ${name} deleted. Its pins, layout and drafts are kept; the id ${id} cannot be used again.`);
      if (id === current) {
        onSwitch(DEFAULT_WORKSPACE);
      } else {
        onChanged();
      }
    } catch (err) {
      setMessage(refusalText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="inspector settings" data-testid="workspaces" aria-label="Workspaces">
      <header>
        <h2>Workspaces</h2>
        <button type="button" onClick={onClose}>
          Close
        </button>
      </header>
      <p className="muted">Each workspace is one application: its own map, pins, layout and drafts.</p>
      <ul className="plain workspace-list" aria-label="Workspaces of this tenant">
        {all.map((w) => (
          <li key={w.id} data-testid={`workspace-${w.id}`}>
            <span className="id">{w.name}</span>
            <span className="kind mono">{w.id}</span>
            {w.id === current && <span className="muted">showing now</span>}
            {w.id !== DEFAULT_WORKSPACE && (
              <Confirm
                ask="Delete…"
                askLabel={`Delete ${w.name}…`}
                confirm={`Delete ${w.id}`}
                keep="Keep it"
                disabled={busy}
                onConfirm={() => void remove(w.id, w.name)}
                explain={
                  <p className="warn">
                    Delete workspace {w.name}? It leaves the workspace menu for everyone. Its pins, layout and drafts are kept, and the id <span className="mono">{w.id}</span> cannot be used again.
                  </p>
                }
                wrap={(question) => (
                  <div className="settings-delete" data-testid="settings-delete-confirm">
                    {question}
                  </div>
                )}
              />
            )}
          </li>
        ))}
      </ul>
      {message && (
        <p className="warn" role="alert">
          {message}
        </p>
      )}
      {creating ? (
        <NewWorkspaceForm
          onCancel={() => setCreating(false)}
          onCreated={(made) => {
            setCreating(false);
            onNotice(`Workspace ${made.name ?? made.id} created.`);
            onSwitch(made.id);
          }}
        />
      ) : (
        <p>
          <button type="button" className="primary" onClick={() => setCreating(true)} data-testid="settings-new-workspace">
            New workspace…
          </button>
        </p>
      )}
    </section>
  );
}
