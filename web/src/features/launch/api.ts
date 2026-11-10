import type { LaunchRequest, LaunchResponse } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const launchApi = {
  launch: (req: LaunchRequest) => request<LaunchResponse>("POST", "/api/launch", req),
};
