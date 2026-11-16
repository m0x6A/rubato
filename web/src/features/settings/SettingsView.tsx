import { useCallback, useEffect, useState } from "react";
import type { Overview, SecretUpdate, SourceField, SourceFields, WorkspaceSettings, WorkspaceSettingsUpdate } from "../../protocol.ts";
import { DEFAULT_WORKSPACE } from "../../shared/workspace.ts";
import { refusalOf, refusalText, settingsApi } from "./api.ts";
import { NewWorkspaceForm } from "./NewWorkspaceForm.tsx";

interface Props {
  overview: Overview;
  onClose: () => void;
  /** A change was stored: the shell refetches the overview. */
  onChanged: () => void;
  onNotice: (text: string) => void;
  /** Show another workspace: after a create, and after this one is deleted. */
  onSwitch: (id: string) => void;
}

type SecretChoice = "keep" | "set" | "clear";

/** What the form holds: text as typed, so an empty box and "0" stay apart. */
interface Draft {
  name: string;
  mapRef: string;
  /** The adapter's name; empty: no traffic source. */
  source: string;
  fields: Record<string, string>;
  secrets: Record<string, { choice: SecretChoice; value: string }>;
  cap: string;
  attempts: string;
}

function draftOf(s: WorkspaceSettings): Draft {
  return {
    name: s.name ?? "",
    mapRef: s.map_ref,
    source: s.traffic?.source ?? "",
    fields: Object.fromEntries(Object.entries(s.traffic?.fields ?? {}).map(([k, v]) => [k, v ?? ""])),
    secrets: {},
    cap: s.default_spend_cap_usd == null ? "" : String(s.default_spend_cap_usd),
    attempts: s.default_attempt_limit == null ? "" : String(s.default_attempt_limit),
  };
}

/** `scheme://host:port` of a URL as typed, or null while it does not read. */
function originOf(url: string | undefined): string | null {
  try {
    return url ? new URL(url).origin : null;
  } catch {
    return null;
  }
}

const INPUT_TYPE: Record<SourceField["kind"], string> = { text: "text", url: "url", number: "number" };

/**
 * Configure workspace (issue #189, ADR 0075): this workspace's name, map source, traffic source
 * and launch defaults, set at runtime. The traffic form is drawn from the field list the
 * adapter publishes; a secret shows only as set or not set. No application setting is here.
 */
export function SettingsView({ overview, onClose, onChanged, onNotice, onSwitch }: Props) {
  const workspace = overview.workspace ?? DEFAULT_WORKSPACE;
  const [settings, setSettings] = useState<WorkspaceSettings | null>(null);
  const [sources, setSources] = useState<SourceFields[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  /** A stale edit: the settings as someone else left them, offered in place of this edit. */
  const [theirs, setTheirs] = useState<WorkspaceSettings | null>(null);
  const [creating, setCreating] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const show = useCallback((s: WorkspaceSettings) => {
    setSettings(s);
    setDraft(draftOf(s));
    setTheirs(null);
  }, []);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const [s, list] = await Promise.all([settingsApi.get(), settingsApi.sources()]);
      setSources(list);
      show(s);
    } catch (e) {
      setLoadError(refusalText(e));
    }
  }, [show]);

  useEffect(() => {
    setSettings(null);
    setDraft(null);
    setMessage(null);
    setSaved(false);
    setConfirmDelete(false);
    void load();
  }, [workspace, load]);

  if (loadError) {
    return (
      <section className="inspector settings" data-testid="settings" aria-label="Configure workspace">
        <header>
          <h2>Configure workspace</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <p role="alert" className="settings-error">
          The settings could not be read: {loadError}
        </p>
        <p>
          <button type="button" onClick={() => void load()}>
            Try again
          </button>
        </p>
      </section>
    );
  }
  if (!settings || !draft) {
    return (
      <section className="inspector settings" data-testid="settings" aria-label="Configure workspace" aria-busy="true">
        <header>
          <h2>Configure workspace</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <p className="muted">Loading the settings…</p>
      </section>
    );
  }

  const edit = (change: Partial<Draft>) => {
    setDraft({ ...draft, ...change });
    setSaved(false);
  };
  const adapter = sources.find((s) => s.source === draft.source);
  const storedSecrets = settings.traffic?.source === draft.source ? (settings.traffic.secrets ?? {}) : {};
  const isSet = (name: string) => storedSecrets[name]?.set === true;
  const choiceOf = (name: string) => draft.secrets[name] ?? { choice: "keep" as SecretChoice, value: "" };
  // A stored secret belongs to the address it was set for (ADR 0075).
  const originMoved = settings.traffic?.source === draft.source && originOf(settings.traffic?.fields?.["url"]) !== originOf(draft.fields["url"]);

  const number = (text: string, what: string): number | null => {
    if (!text.trim()) {
      return null;
    }
    const n = Number(text);
    if (!Number.isFinite(n)) {
      throw new Error(`${what} is a number.`);
    }
    return n;
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (busy) {
      return;
    }
    let update: WorkspaceSettingsUpdate;
    try {
      const fields: Record<string, string> = {};
      const secrets: Record<string, SecretUpdate> = {};
      for (const f of adapter?.fields ?? []) {
        if (f.secret) {
          const c = choiceOf(f.name);
          secrets[f.name] = c.choice === "set" ? { action: "set", value: c.value } : { action: c.choice };
        } else if ((draft.fields[f.name] ?? "").trim()) {
          fields[f.name] = (draft.fields[f.name] ?? "").trim();
        }
      }
      update = {
        revision: settings.revision,
        name: draft.name.trim() || null,
        map_repo: settings.map_repo,
        map_path: settings.map_path,
        map_ref: draft.mapRef.trim(),
        traffic: draft.source ? { source: draft.source, fields, secrets } : null,
        default_spend_cap_usd: number(draft.cap, "The spend cap"),
        default_attempt_limit: number(draft.attempts, "The attempt limit"),
      };
    } catch (err) {
      setMessage(err instanceof Error ? err.message : String(err));
      return;
    }
    setBusy(true);
    setMessage(null);
    setSaved(false);
    try {
      show(await settingsApi.update(update));
      setSaved(true);
      onChanged();
    } catch (err) {
      const refusal = refusalOf(err);
      if (refusal?.reason === "changed_meanwhile" && refusal.current) {
        setTheirs(refusal.current);
        setMessage(`${refusal.current.updated_by} changed these settings while you were editing. Your edit was not saved.`);
      } else {
        setMessage(refusalText(err));
      }
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    setMessage(null);
    try {
      await settingsApi.remove(workspace);
      onNotice(`Workspace ${settings.name ?? workspace} deleted. Its pins, layout and drafts are kept; the id ${workspace} cannot be used again.`);
      onSwitch(DEFAULT_WORKSPACE);
      onClose();
    } catch (err) {
      setMessage(refusalText(err));
      setConfirmDelete(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="inspector settings" data-testid="settings" aria-label="Configure workspace">
      <header>
        <h2>Configure workspace</h2>
        <span className="kind mono">{workspace}</span>
        <button type="button" onClick={onClose}>
          Close
        </button>
      </header>
      <form onSubmit={(e) => void submit(e)} aria-label="Workspace settings">
        <label>
          Name
          <input value={draft.name} onChange={(e) => edit({ name: e.target.value })} placeholder={overview.map.map.name} maxLength={80} data-testid="settings-name" />
        </label>

        <fieldset>
          <legend>Map</legend>
          <dl>
            <dt>Repo</dt>
            <dd className="mono">{settings.map_repo}</dd>
            <dt>Path</dt>
            <dd className="mono">{settings.map_path}</dd>
          </dl>
          <p className="muted">The repo and path are fixed: this workspace's pins, layout and drafts belong to that map.</p>
          <label>
            Ref
            <input value={draft.mapRef} onChange={(e) => edit({ mapRef: e.target.value })} required data-testid="settings-map-ref" />
          </label>
        </fieldset>

        <fieldset>
          <legend>Traffic source</legend>
          <label>
            Source
            <select value={draft.source} onChange={(e) => edit({ source: e.target.value, secrets: {} })} data-testid="settings-source">
              <option value="">None</option>
              {sources.map((s) => (
                <option key={s.source} value={s.source}>
                  {s.label}
                </option>
              ))}
              {draft.source && !adapter && <option value={draft.source}>{draft.source}</option>}
            </select>
          </label>
          {(adapter?.fields ?? []).map((f) =>
            f.secret ? (
              <div className="settings-secret" key={f.name} data-testid={`settings-secret-${f.name}`}>
                <label>
                  {f.label}
                  <span className="muted" data-testid={`settings-secret-state-${f.name}`}>
                    {isSet(f.name) ? "set" : "not set"}
                  </span>
                  <select
                    aria-label={`${f.label}: keep, replace or clear`}
                    value={choiceOf(f.name).choice}
                    onChange={(e) => edit({ secrets: { ...draft.secrets, [f.name]: { choice: e.target.value as SecretChoice, value: "" } } })}
                  >
                    <option value="keep">{isSet(f.name) ? "Keep" : "Leave unset"}</option>
                    <option value="set">{isSet(f.name) ? "Replace" : "Set"}</option>
                    {isSet(f.name) && <option value="clear">Clear</option>}
                  </select>
                </label>
                {choiceOf(f.name).choice === "set" && (
                  <input
                    type="password"
                    autoComplete="off"
                    aria-label={`${f.label}: new value`}
                    value={choiceOf(f.name).value}
                    placeholder={f.placeholder ?? ""}
                    onChange={(e) => edit({ secrets: { ...draft.secrets, [f.name]: { choice: "set", value: e.target.value } } })}
                  />
                )}
                {isSet(f.name) && choiceOf(f.name).choice === "keep" && originMoved && (
                  <p className="warn" data-testid={`settings-secret-cleared-${f.name}`}>
                    The address changed, so the stored value will be cleared. Choose Replace to set it for the new address.
                  </p>
                )}
              </div>
            ) : (
              <label key={f.name}>
                {f.label}
                <input
                  type={INPUT_TYPE[f.kind]}
                  value={draft.fields[f.name] ?? ""}
                  required={f.required}
                  placeholder={f.placeholder ?? ""}
                  onChange={(e) => edit({ fields: { ...draft.fields, [f.name]: e.target.value } })}
                  data-testid={`settings-field-${f.name}`}
                />
              </label>
            ),
          )}
        </fieldset>

        <fieldset>
          <legend>Launch defaults</legend>
          <label>
            Spend cap, USD
            <input type="number" min="0" step="any" value={draft.cap} placeholder="25" onChange={(e) => edit({ cap: e.target.value })} data-testid="settings-cap" />
          </label>
          <label>
            Attempt limit
            <input type="number" min="1" step="1" value={draft.attempts} placeholder="3" onChange={(e) => edit({ attempts: e.target.value })} data-testid="settings-attempts" />
          </label>
          <p className="muted">A plan's own cap and limit still win.</p>
        </fieldset>

        {message && (
          <p role="alert" className="settings-error" data-testid="settings-message">
            {message}
          </p>
        )}
        {theirs && (
          <p>
            <button type="button" onClick={() => { show(theirs); setMessage(null); }}>
              Show their version
            </button>
          </p>
        )}
        <p className="settings-actions">
          <button type="submit" disabled={busy} data-testid="settings-save">
            {busy ? "Saving…" : "Save"}
          </button>
          <span role="status" className="muted">
            {saved ? "Saved." : ""}
          </span>
        </p>
        <p className="muted">
          Last changed by {settings.updated_by}, revision {settings.revision}.
        </p>
      </form>

      <h3>Workspaces</h3>
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
          <button type="button" onClick={() => setCreating(true)} data-testid="settings-new-workspace">
            New workspace…
          </button>
        </p>
      )}
      {workspace !== DEFAULT_WORKSPACE &&
        (confirmDelete ? (
          <div className="settings-delete" data-testid="settings-delete-confirm">
            <p className="warn">
              Delete workspace {settings.name ?? workspace}? It leaves the switcher for everyone. Its pins, layout and drafts are kept, and the id <span className="mono">{workspace}</span> cannot be used again.
            </p>
            <p className="settings-actions">
              <button type="button" disabled={busy} onClick={() => void remove()}>
                Delete {workspace}
              </button>
              <button type="button" onClick={() => setConfirmDelete(false)}>
                Keep it
              </button>
            </p>
          </div>
        ) : (
          <p>
            <button type="button" onClick={() => setConfirmDelete(true)} data-testid="settings-delete">
              Delete this workspace…
            </button>
          </p>
        ))}
    </section>
  );
}
