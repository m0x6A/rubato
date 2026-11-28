import type { AdminInstallation, AdminTenant, NewTenant, PurgeReport } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

/** The operator's routes (ADR 0087, 0088). They are under no tenant: an operator need be in none. */
export const adminApi = {
  installations: () => request<AdminInstallation[]>("GET", "/api/admin/installations"),
  tenants: () => request<AdminTenant[]>("GET", "/api/admin/tenants"),
  createTenant: (t: NewTenant) => request<AdminTenant>("POST", "/api/admin/tenants", t),
  deleteTenant: (id: string) => request<void>("DELETE", `/api/admin/tenants/${encodeURIComponent(id)}`),
  attach: (tenant: string, installation: number) => request<void>("POST", `/api/admin/tenants/${encodeURIComponent(tenant)}/installations/${installation}`),
  detach: (tenant: string, installation: number) => request<void>("DELETE", `/api/admin/tenants/${encodeURIComponent(tenant)}/installations/${installation}`),
  /** Without `confirm`: what a purge would delete. With it: deleted. */
  purge: (installation: number, confirm: boolean) => request<PurgeReport>("POST", `/api/admin/installations/${installation}/purge${confirm ? "?confirm=true" : ""}`),
  nameAdmin: (tenant: string, login: string) => request<void>("POST", `/api/admin/tenants/${encodeURIComponent(tenant)}/admins`, { login }),
};

const WORDS: Record<string, (detail?: string | null) => string> = {
  bad_id: () => "A tenant id is 1 to 32 lower-case letters, digits and hyphens, and a tenant has a name.",
  duplicate: () => "A tenant with that id exists.",
  id_has_rows: () => "Rows of a deleted tenant with that id are still kept. Purge its detached installations first.",
  unknown_login: () => "GitHub has no account with that login.",
  unknown_installation: () => "The App has no such installation, or it is not attached to that tenant.",
  installation_taken: (d) => `That installation is attached to tenant ${d ?? "another tenant"}. Detach it there first.`,
  rows_of_other_tenant: (d) => `Rows of that installation's repositories are still kept for tenant ${d ?? "another tenant"}. Purge them first.`,
  installation_in_use: (d) => `A ${d ?? "workspace"} of the tenant is on one of its repositories. Delete that workspace first.`,
  tenant_not_empty: (d) => `The tenant is not empty: ${d ?? "it has a workspace or an attached installation"}.`,
  not_detached: () => "The installation is attached. Detach it before purging.",
  last_admin: () => "A tenant keeps at least one admin.",
  lookup_failed: (d) => `GitHub could not be asked. Nothing was changed.${d ? ` (${d})` : ""}`,
  not_operator: () => "Only an operator of this Rubato can do that.",
};

/** A refusal's reason in words, with what to do about it. */
export function words(reason: string, detail?: string | null): string {
  return WORDS[reason]?.(detail) ?? reason;
}
