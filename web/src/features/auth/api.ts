import type { Me } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const authApi = {
  me: () => request<Me>("GET", "/api/me"),
  /** Ends this browser's session; the server answers 204 and clears the cookie. */
  logout: () => request<void>("POST", "/auth/logout"),
};
