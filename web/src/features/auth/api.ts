import type { Me } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const authApi = {
  me: () => request<Me>("GET", "/api/me"),
};
