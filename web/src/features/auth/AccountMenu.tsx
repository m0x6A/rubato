import { useState } from "react";
import type { Me } from "../../protocol.ts";
import { failureText } from "../../shared/api.ts";
import { Menu } from "../../shared/Menu.tsx";
import { authApi } from "./api.ts";

interface Props {
  me: Me;
  /** The tenant the page shows. */
  tenant: string | null;
  onSwitchTenant: (id: string) => void;
  /** The members of the tenant shown; absent for a person in no tenant. */
  onMembers?: (() => void) | undefined;
  /** The operator's page; absent for everyone else. */
  onOperator?: (() => void) | undefined;
  onNotice: (text: string) => void;
}

/**
 * Who is logged in, and what is theirs to reach (issue #357): the tenants they are in, the
 * members of the one shown, the operator's page, and logging out.
 */
export function AccountMenu({ me, tenant, onSwitchTenant, onMembers, onOperator, onNotice }: Props) {
  const [leaving, setLeaving] = useState(false);
  const tenants = me.tenants ?? [];
  const logOut = async () => {
    setLeaving(true);
    try {
      await authApi.logout();
      // The session is gone: start over, at the login page.
      location.assign("/");
    } catch (e) {
      setLeaving(false);
      onNotice(`You were not logged out: ${failureText(e)}`);
    }
  };
  return (
    <span className="top-menu" data-testid="account-menu">
      <Menu
        button={
          <>
            <span className="me">{me.login}</span> <span aria-hidden="true">▾</span>
          </>
        }
        buttonLabel={`Account ${me.login}`}
        menuLabel="Account"
      >
        {(close) => (
          <>
            {tenants.length > 1 && (
              <div role="group" aria-label="Show another tenant">
                <div className="pin-menu-heading" aria-hidden="true">
                  Tenant
                </div>
                {tenants.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    role="menuitemradio"
                    aria-checked={t.id === tenant}
                    tabIndex={-1}
                    data-testid={`switch-tenant-${t.id}`}
                    onClick={() => {
                      close();
                      if (t.id !== tenant) {
                        onSwitchTenant(t.id);
                      }
                    }}
                  >
                    <span>{t.name}</span>
                    <span className="muted">{t.id === tenant ? "showing" : ""}</span>
                  </button>
                ))}
              </div>
            )}
            <div role="group" aria-label="Pages" className="menu-group">
              {onMembers && (
                <button
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  data-testid="open-members"
                  onClick={() => {
                    close();
                    onMembers();
                  }}
                >
                  Members
                </button>
              )}
              {onOperator && (
                <button
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  data-testid="open-operator"
                  onClick={() => {
                    close();
                    onOperator();
                  }}
                >
                  Operator
                </button>
              )}
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                data-testid="log-out"
                aria-disabled={leaving || undefined}
                onClick={() => {
                  close();
                  void logOut();
                }}
              >
                Log out
              </button>
            </div>
          </>
        )}
      </Menu>
    </span>
  );
}
