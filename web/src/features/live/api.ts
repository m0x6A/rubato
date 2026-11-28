import type { Overview, WorkspaceInfo } from "../../protocol.ts";
import { ApiError, request } from "../../shared/api.ts";
import { currentTenant, currentWorkspace, forgetWorkspace, inTenant, isUnknownWorkspace, learnWorkspace, scoped } from "../../shared/workspace.ts";

/** A workspace this page asked for that the server does not hold, once, for the shell to say so. */
let forgotten: string | null = null;

export function takeForgottenWorkspace(): string | null {
  const id = forgotten;
  forgotten = null;
  return id;
}

export const liveApi = {
  /**
   * The overview of this page's workspace. Learns which workspace it is, so later map calls are
   * scoped to it. A workspace the tenant does not hold (any more) is forgotten, and the first
   * of the tenant's workspaces is read instead (ADR 0049, ADR 0089); a tenant with none says so.
   */
  overview: async (): Promise<Overview> => {
    const read = async (): Promise<Overview> => {
      const ov = await request<Overview>("GET", scoped("/api/overview"));
      learnWorkspace(ov.workspace ?? "default");
      return ov;
    };
    try {
      return await read();
    } catch (e) {
      if (!(e instanceof ApiError && isUnknownWorkspace(e.status, e.message))) {
        throw e;
      }
      const asked = currentWorkspace();
      forgetWorkspace();
      if (currentTenant() === null) {
        // A server from before tenants: unprefixed is its `default`.
        forgotten = asked;
        return await read();
      }
      const held = await request<WorkspaceInfo[]>("GET", inTenant("/api/workspaces"));
      const first = held[0];
      if (!first) {
        throw new ApiError(404, "This tenant has no workspace yet. One of its admins creates the first.");
      }
      // Only a workspace the page asked for by name is reported as gone.
      forgotten = asked;
      learnWorkspace(first.id);
      return await read();
    }
  },
};
