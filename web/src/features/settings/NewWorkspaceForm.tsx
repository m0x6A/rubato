import { useState } from "react";
import type { WorkspaceSettings } from "../../protocol.ts";
import { refusalText, settingsApi } from "./api.ts";

/**
 * A new workspace (ADR 0075): an id, a name and a map the person can read on GitHub. Its traffic
 * source and launch defaults are set afterwards, in the workspace's own settings.
 */
export function NewWorkspaceForm({ onCreated, onCancel }: { onCreated: (made: WorkspaceSettings) => void; onCancel: () => void }) {
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [repo, setRepo] = useState("");
  const [path, setPath] = useState("system-map.yml");
  const [ref, setRef] = useState("main");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (busy) {
      return;
    }
    setBusy(true);
    setMessage(null);
    try {
      onCreated(await settingsApi.create({ id: id.trim(), name: name.trim() || null, map_repo: repo.trim(), map_path: path.trim(), map_ref: ref.trim() || "main" }));
    } catch (err) {
      setMessage(refusalText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="settings-new" onSubmit={(e) => void submit(e)} aria-label="New workspace" data-testid="new-workspace">
      <label>
        Id
        <input value={id} onChange={(e) => setId(e.target.value)} required pattern="[a-z0-9\-]{1,32}" title="1 to 32 of a-z, 0-9 and -" placeholder="shop" />
      </label>
      <label>
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={80} placeholder="The map's own name" />
      </label>
      <label>
        Map repo
        <input value={repo} onChange={(e) => setRepo(e.target.value)} required placeholder="owner/name" />
      </label>
      <label>
        Map path
        <input value={path} onChange={(e) => setPath(e.target.value)} required />
      </label>
      <label>
        Map ref
        <input value={ref} onChange={(e) => setRef(e.target.value)} required />
      </label>
      <p className="muted">You must be able to read the map's repo, and every repo the map names, on GitHub. The id, the repo and the path cannot be changed later.</p>
      {message && (
        <p role="alert" className="settings-error" data-testid="new-workspace-message">
          {message}
        </p>
      )}
      <p className="settings-actions">
        <button type="submit" disabled={busy}>
          {busy ? "Creating…" : "Create workspace"}
        </button>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </p>
    </form>
  );
}
