import { useCallback, useEffect, useState } from "react";
import { Confirm } from "../../shared/Confirm.tsx";
import type { MemberView, Role, TenantInfo, WebEvent } from "../../protocol.ts";
import { onWebEvent } from "../live/index.ts";
import { membersApi, refusalText } from "./api.ts";

interface Props {
  /** The tenant shown, with the person's own role in it. */
  tenant: TenantInfo;
  /** The person's own login, so their own row says "you". */
  me: string;
  onClose: () => void;
  onNotice: (text: string) => void;
}

/**
 * A tenant's members (ADR 0088). Every member sees who is in the tenant; an admin invites by
 * GitHub login, changes roles and removes. The invite alone is the gate, so the view says what an
 * invited person can then do.
 */
export function MembersView({ tenant, me, onClose, onNotice }: Props) {
  const admin = tenant.role === "admin";
  const [members, setMembers] = useState<MemberView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [login, setLogin] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [busy, setBusy] = useState(false);
  /** The member whose removal waits for a second press. */

  const load = useCallback(() => {
    membersApi
      .list()
      .then((m) => {
        setMembers(m);
        setError(null);
      })
      .catch((e: unknown) => setError(refusalText(e)));
  }, []);

  useEffect(() => {
    load();
    // Someone else changed the members: the list follows.
    return onWebEvent((ev: WebEvent) => {
      if (ev.type === "changed" && ev.reason === "members") {
        load();
      }
    });
  }, [load]);

  /** Runs one change; `what` answers with the notice to show. */
  const run = async (what: () => Promise<string>) => {
    setBusy(true);
    try {
      const done = await what();
      setError(null);
      onNotice(done);
      load();
    } catch (e) {
      setError(refusalText(e));
    } finally {
      setBusy(false);
    }
  };

  const invite = (e: React.FormEvent) => {
    e.preventDefault();
    const who = login.trim();
    if (!who) {
      return;
    }
    void run(async () => {
      const m = await membersApi.invite({ login: who, role });
      setLogin("");
      return `${m.login} (GitHub user ${m.github_id}) was invited to ${tenant.name} as ${m.role}`;
    });
  };

  return (
    <section className="inspector settings" data-testid="members" aria-label={`Members of ${tenant.name}`}>
      <div className="settings-actions">
        <h2>Members of {tenant.name}</h2>
        <button type="button" onClick={onClose}>
          Close
        </button>
      </div>
      {error && (
        <p className="settings-error" role="alert">
          {error}
        </p>
      )}
      {members === null && !error && <p className="muted">Loading the members…</p>}
      {members && (
        <ul className="members" data-testid="member-list">
          {members.map((m) => (
            <li key={m.github_id} data-testid={`member-${m.login}`}>
              <span>
                <strong>{m.login}</strong>
                {m.login.toLowerCase() === me.toLowerCase() && <span className="muted"> (you)</span>}
                {!m.joined && <span className="muted" title={`Invited by ${m.invited_by}; has not logged in yet`}> · invited</span>}
              </span>
              {admin ? (
                <select value={m.role} disabled={busy} aria-label={`Role of ${m.login}`} onChange={(e) => {
                    const to = e.target.value as Role;
                    void run(() => membersApi.setRole(m.github_id, to).then(() => `${m.login} is now ${to}`));
                  }}>
                  <option value="admin">admin</option>
                  <option value="member">member</option>
                </select>
              ) : (
                <span className="kind mono">{m.role}</span>
              )}
              {admin && (
                <Confirm
                  ask="Remove…"
                  askLabel={`Remove ${m.login}`}
                  confirm={`Remove ${m.login}`}
                  confirmTestId={`confirm-remove-${m.login}`}
                  keep="Keep"
                  disabled={busy}
                  onConfirm={() => void run(() => membersApi.remove(m.github_id).then(() => `${m.login} was removed from ${tenant.name}`))}
                />
              )}
            </li>
          ))}
        </ul>
      )}
      {admin ? (
        <form onSubmit={invite} data-testid="invite-form">
          <fieldset>
            <legend>Invite by GitHub login</legend>
            <label>
              GitHub login
              <input value={login} onChange={(e) => setLogin(e.target.value)} placeholder="octocat" autoComplete="off" spellCheck={false} />
            </label>
            <label>
              Role
              <select value={role} onChange={(e) => setRole(e.target.value as Role)}>
                <option value="member">member</option>
                <option value="admin">admin</option>
              </select>
            </label>
            <p className="muted">
              The invite is the only gate. Whatever their own access on GitHub, a member reads every open issue, pull request, plan, room and context graph source of this tenant's repositories, private ones included, and can create issues and propose map changes, which Rubato writes as its GitHub App. Approving a plan and merging use their own GitHub account, so GitHub decides those.
            </p>
            <div className="settings-actions">
              <button type="submit" disabled={busy || !login.trim()}>
                Invite
              </button>
            </div>
          </fieldset>
        </form>
      ) : (
        <p className="muted">An admin of {tenant.name} invites and removes members.</p>
      )}
    </section>
  );
}
