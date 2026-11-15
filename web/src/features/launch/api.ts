import type { LaunchRequest, LaunchResponse } from "../../protocol.ts";
import { request } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

export const launchApi = {
  launch: (req: LaunchRequest) => request<LaunchResponse>("POST", scoped("/api/launch"), req),
};
