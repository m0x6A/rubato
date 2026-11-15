import type { DiscrepancyKey, MapLayout, TrafficDismissal } from "../../protocol.ts";
import { request } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

export const mapApi = {
  /** Store the parts that moved. */
  putLayout: (moved: MapLayout) => request<void>("PUT", scoped("/api/layout"), moved),
  /** Remove every stored position for the team; the answer is what was removed. */
  clearLayout: () => request<MapLayout>("DELETE", scoped("/api/layout")),
  /** Remove the By repo arrangement for the team (ADR 0068); the flat map stays. The answer is what was removed. */
  clearGrouped: () => request<MapLayout>("DELETE", scoped("/api/layout/grouped")),
  /** Dismiss a discrepancy for the workspace, as yourself (ADR 0060). */
  dismiss: (target: DiscrepancyKey) => request<TrafficDismissal>("POST", scoped("/api/traffic/dismissals"), { target }),
  /** Undo a dismissal. */
  undismiss: (id: number) => request<void>("DELETE", scoped(`/api/traffic/dismissals/${id}`)),
};
