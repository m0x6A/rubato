import type { MapLayout } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const mapApi = {
  /** Store the parts that moved. */
  putLayout: (moved: MapLayout) => request<void>("PUT", "/api/layout", moved),
  /** Remove every stored position for the team; the answer is what was removed. */
  clearLayout: () => request<MapLayout>("DELETE", "/api/layout"),
};
