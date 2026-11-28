import { useCallback, useEffect, useState } from "react";
import type { AdminInstallation, AdminTenant, PurgeReport } from "../../protocol.ts";
import { failureText } from "../../shared/api.ts";
import { refusalOf } from "../tenants/index.ts";
import { adminApi, words } from "./api.ts";

interface Props {
  onClose: () => void;
  onNotice: (text: string) => void;
}

function textOf(e: unknown): string {
  const r = refusalOf(e);
  return r ? words(r.reason, r.detail) : failureText(e);
}

const STATE: Record<AdminInstallation["state"], string> = {
  unattached: "not attached",
  attached: "attached",
  suspended: "suspended on GitHub",
  detached: "detached, rows kept",
};

/**
 * The operator's page (ADR 0087, 0088): the GitHub App's installations and the tenants. An
 * operator creates a tenant with its first admin, attaches and detaches installations, purges a
 * detached one and names an admin. It shows names and counts, never a tenant's data.
 */
export function AdminView({ onClose, onNotice }: Props) {
  const [installations, setInstallations] = useState<AdminInstallation[] | null>(null);
  const [installationsError, setInstallationsError] = useState<string | null>(null);
  const [tenants, setTenants] = useState<AdminTenant[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [fresh, setFresh] = useState({ id: "", name: "", admin: "" });
  /** Which tenant each unattached installation would be attached to. */
  const [target, setTarget] = useState<Record<number, string>>({});
  const [newAdmin, setNewAdmin] = useState<Record<string, string>>({});
  /** A purge that was counted and waits for the second press. */
  const [purge, setPurge] = useState<PurgeReport | null>(null);

  const load = useCallback(() => {
    adminApi
      .tenants()
      .then(setTenants)
      .catch((e: unknown) => setError(textOf(e)));
    // The tenants still show when GitHub cannot list the installations.
    adminApi
      .installations()
      .then((i) => {
        setInstallations(i);
        setInstallationsError(null);
      })
      .catch((e: unknown) => setInstallationsError(textOf(e)));
  }, []);
  useEffect(load, [load]);

  const run = async (what: () => Promise<string>) => {
    setBusy(true);
    try {
      const done = await what();
      setError(null);
      onNotice(done);
      load();
    } catch (e) {
      setError(textOf(e));
    } finally {
      setBusy(false);
    }
  };

  const create = (e: React.FormEvent) => {
    e.preventDefault();
    void run(async () => {
      const t = await adminApi.createTenant({ id: fresh.id.trim(), name: fresh.name.trim(), first_admin: fresh.admin.trim(), installations: [] });
      setFresh({ id: "", name: "", admin: "" });
      return `Tenant ${t.name} was created with ${fresh.admin.trim()} as its admin`;
    });
  };

  const rowsOf = (r: PurgeReport) => Object.values(r.rows).reduce<number>((a, b) => a + (b ?? 0), 0);

  return (
    <section className="inspector settings" data-testid="operator" aria-label="Operator">
      <div className="settings-actions">
        <h2>Operator</h2>
        <button type="button" onClick={onClose}>
          Close
        </button>
      </div>
      <p className="muted">Tenants, the GitHub App's installations and who administers each tenant. A tenant's data is its members' to see, not the operator's.</p>
      {error && (
        <p className="settings-error" role="alert">
          {error}
        </p>
      )}

      <fieldset>
        <legend>Tenants</legend>
        {tenants === null && !error && <p className="muted">Loading the tenants…</p>}
        {tenants?.length === 0 && <p className="muted">No tenant yet. Create the first one below.</p>}
        <ul className="members" data-testid="tenant-list">
          {(tenants ?? []).map((t) => (
            <li key={t.id} data-testid={`tenant-${t.id}`}>
              <span>
                <strong>{t.name}</strong> <span className="mono muted">{t.id}</span>
                <br />
                <span className="muted">
                  {t.members} {t.members === 1 ? "member" : "members"}, {t.admins} {t.admins === 1 ? "admin" : "admins"} · {t.workspaces} {t.workspaces === 1 ? "workspace" : "workspaces"} · {t.installations.length === 0 ? "no installation" : `installation ${t.installations.join(", ")}`}
                </span>
              </span>
              <span className="settings-actions">
                <input aria-label={`GitHub login of a new admin of ${t.name}`} placeholder="login of a new admin" value={newAdmin[t.id] ?? ""} onChange={(e) => setNewAdmin((n) => ({ ...n, [t.id]: e.target.value }))} autoComplete="off" spellCheck={false} />
                <button
                  type="button"
                  disabled={busy || !(newAdmin[t.id] ?? "").trim()}
                  onClick={() => {
                    const who = (newAdmin[t.id] ?? "").trim();
                    void run(async () => {
                      await adminApi.nameAdmin(t.id, who);
                      setNewAdmin((n) => ({ ...n, [t.id]: "" }));
                      return `${who} is an admin of ${t.name}`;
                    });
                  }}
                >
                  Name admin
                </button>
                <button type="button" disabled={busy || t.workspaces > 0 || t.installations.length > 0} title={t.workspaces > 0 || t.installations.length > 0 ? "Only a tenant with no workspace and no attached installation can be deleted" : undefined} onClick={() => void run(() => adminApi.deleteTenant(t.id).then(() => `Tenant ${t.name} was deleted`))}>
                  Delete
                </button>
              </span>
            </li>
          ))}
        </ul>
      </fieldset>

      <form onSubmit={create} data-testid="new-tenant">
        <fieldset>
          <legend>New tenant</legend>
          <label>
            Id
            <input value={fresh.id} onChange={(e) => setFresh((f) => ({ ...f, id: e.target.value }))} placeholder="acme" pattern="[a-z0-9-]{1,32}" title="1 to 32 lower-case letters, digits and hyphens" autoComplete="off" spellCheck={false} />
          </label>
          <label>
            Name
            <input value={fresh.name} onChange={(e) => setFresh((f) => ({ ...f, name: e.target.value }))} placeholder="Acme" autoComplete="off" />
          </label>
          <label>
            GitHub login of its first admin
            <input value={fresh.admin} onChange={(e) => setFresh((f) => ({ ...f, admin: e.target.value }))} placeholder="octocat" autoComplete="off" spellCheck={false} />
          </label>
          <div className="settings-actions">
            <button type="submit" disabled={busy || !fresh.id.trim() || !fresh.name.trim() || !fresh.admin.trim()}>
              Create tenant
            </button>
          </div>
        </fieldset>
      </form>

      <fieldset>
        <legend>Installations of the GitHub App</legend>
        {installationsError && (
          <p className="settings-error" role="alert">
            {installationsError}
          </p>
        )}
        {installations === null && !installationsError && <p className="muted">Loading the installations…</p>}
        {installations?.length === 0 && <p className="muted">The App is installed nowhere yet. Install it on a GitHub account or organisation, then attach it to a tenant here.</p>}
        <ul className="members" data-testid="installation-list">
          {(installations ?? []).map((i) => (
            <li key={i.installation_id} data-testid={`installation-${i.installation_id}`} data-state={i.state}>
              <span>
                <strong>{i.account_login || `installation ${i.installation_id}`}</strong> <span className="muted">{i.account_type === "Organization" ? "organisation" : "account"}</span>
                <br />
                <span className="muted">
                  {STATE[i.state]}
                  {i.tenant ? ` · tenant ${i.tenant}` : ""}
                </span>
              </span>
              {i.state === "unattached" && (
                <span className="settings-actions">
                  <select aria-label={`Tenant to attach ${i.account_login} to`} value={target[i.installation_id] ?? ""} onChange={(e) => setTarget((t) => ({ ...t, [i.installation_id]: e.target.value }))}>
                    <option value="">attach to…</option>
                    {(tenants ?? []).map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.name}
                      </option>
                    ))}
                  </select>
                  <button type="button" disabled={busy || !target[i.installation_id]} onClick={() => void run(() => adminApi.attach(target[i.installation_id] as string, i.installation_id).then(() => `${i.account_login} was attached to ${target[i.installation_id]}`))}>
                    Attach
                  </button>
                </span>
              )}
              {(i.state === "attached" || i.state === "suspended") && i.tenant && (
                <button type="button" disabled={busy} onClick={() => void run(() => adminApi.detach(i.tenant as string, i.installation_id).then(() => `${i.account_login} was detached from ${i.tenant}; its rows are kept until purged`))}>
                  Detach
                </button>
              )}
              {i.state === "detached" &&
                (purge?.installation_id === i.installation_id ? (
                  <span className="settings-delete" data-testid="purge-confirm">
                    <span>
                      Purging deletes {rowsOf(purge)} {rowsOf(purge) === 1 ? "row" : "rows"} of {purge.repos.length} {purge.repos.length === 1 ? "repository" : "repositories"} in tenant {purge.tenant}
                      {Object.keys(purge.rows).length > 0 ? `: ${Object.entries(purge.rows).map(([t, n]) => `${t} ${n}`).join(", ")}` : ""}. This cannot be undone.
                    </span>
                    <span className="settings-actions">
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() =>
                          void run(async () => {
                            const done = await adminApi.purge(i.installation_id, true);
                            setPurge(null);
                            return `Purged ${rowsOf(done)} rows of ${i.account_login || i.installation_id}`;
                          })
                        }
                      >
                        Delete these rows
                      </button>
                      <button type="button" onClick={() => setPurge(null)}>
                        Keep them
                      </button>
                    </span>
                  </span>
                ) : (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      setBusy(true);
                      adminApi
                        .purge(i.installation_id, false)
                        .then((r) => {
                          setPurge(r);
                          setError(null);
                        })
                        .catch((e: unknown) => setError(textOf(e)))
                        .finally(() => setBusy(false));
                    }}
                  >
                    Purge…
                  </button>
                ))}
            </li>
          ))}
        </ul>
      </fieldset>
    </section>
  );
}
