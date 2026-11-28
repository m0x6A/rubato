import type { TenantInfo } from "../../protocol.ts";

/**
 * Which tenant the page shows (ADR 0087): one choice per tenant the person is in. Not drawn for
 * a person in one tenant, who has nothing to choose.
 */
export function TenantSwitcher({ tenants, current, onSwitch }: { tenants: TenantInfo[]; current: string | null; onSwitch: (id: string) => void }) {
  if (tenants.length < 2) {
    return null;
  }
  return (
    <label className="workspace-switch" data-testid="tenant-switch">
      <span className="muted">Tenant</span>
      <select value={current ?? ""} onChange={(e) => onSwitch(e.target.value)} aria-label="Tenant">
        {tenants.map((t) => (
          <option key={t.id} value={t.id}>
            {t.name}
          </option>
        ))}
      </select>
    </label>
  );
}
