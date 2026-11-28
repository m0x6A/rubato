import type { NewWorkspace, SettingsRefusal, SourceFields, WorkspaceSettings, WorkspaceSettingsUpdate } from "../../protocol.ts";
import { ApiError, failureText, request } from "../../shared/api.ts";
import { inTenant, scoped, workspacePath } from "../../shared/workspace.ts";

export const settingsApi = {
  /** The settings of this page's workspace. */
  get: () => request<WorkspaceSettings>("GET", scoped("/api/settings")),
  /** Every traffic adapter's field list: what the form is drawn from. */
  sources: () => request<SourceFields[]>("GET", inTenant("/api/traffic/sources")),
  update: (u: WorkspaceSettingsUpdate) => request<WorkspaceSettings>("PUT", scoped("/api/settings"), u),
  create: (n: NewWorkspace) => request<WorkspaceSettings>("POST", inTenant("/api/workspaces"), n),
  /** Always by id: a workspace is never deleted through the unprefixed routes. */
  remove: (id: string) => request<void>("DELETE", workspacePath(id)),
};

/** The server's refusal of a settings write, when the failure carries one. */
export function refusalOf(e: unknown): SettingsRefusal | null {
  if (!(e instanceof ApiError)) {
    return null;
  }
  try {
    const body = JSON.parse(e.message) as Partial<SettingsRefusal>;
    return typeof body.reason === "string" && typeof body.message === "string" ? (body as SettingsRefusal) : null;
  } catch {
    return null;
  }
}

/** What to tell the person: the refusal's own words, else what the call answered. */
export function refusalText(e: unknown): string {
  return refusalOf(e)?.message ?? failureText(e);
}
