import type { Overview } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const liveApi = {
  overview: () => request<Overview>("GET", "/api/overview"),
};
