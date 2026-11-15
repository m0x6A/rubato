import type { Overview } from "../../protocol.ts";
import { ApiError, request } from "../../shared/api.ts";
import { currentWorkspace, forgetWorkspace, isUnknownWorkspace, learnWorkspace, scoped } from "../../shared/workspace.ts";

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
   * scoped to it; a workspace the server no longer holds is forgotten and the overview of
   * `default` is read instead (ADR 0049).
   */
  overview: async (): Promise<Overview> => {
    try {
      const ov = await request<Overview>("GET", scoped("/api/overview"));
      learnWorkspace(ov.workspace ?? "default");
      return ov;
    } catch (e) {
      if (!(e instanceof ApiError && isUnknownWorkspace(e.status, e.message))) {
        throw e;
      }
      forgotten = currentWorkspace();
      forgetWorkspace();
      const ov = await request<Overview>("GET", "/api/overview");
      learnWorkspace(ov.workspace ?? "default");
      return ov;
    }
  },
};
