import type { DraftOpRequest, ProposeResponse } from "../../protocol.ts";
import { request, requestText } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

/** One change to the map, as the draft stores it. */
export type MapOp = DraftOpRequest;

export const draftsApi = {
  /** Add an operation. The id made here lets a retried request add it once. */
  addOp: (op: MapOp) => request<void>("POST", scoped("/api/map/draft/ops"), { ...op, op_id: crypto.randomUUID() }),
  removeOp: (seq: number) => request<void>("DELETE", scoped(`/api/map/draft/ops/${seq}`)),
  discard: () => request<void>("POST", scoped("/api/map/draft/discard")),
  restore: () => request<void>("POST", scoped("/api/map/draft/restore")),
  /** The file a proposal would commit; with `base`, the loaded map written the same way. */
  yaml: (base = false) => requestText("GET", scoped(`/api/map/draft/yaml${base ? "?base=true" : ""}`)),
  propose: () => request<ProposeResponse>("POST", scoped("/api/map/draft/propose")),
  withdraw: () => request<void>("POST", scoped("/api/map/draft/withdraw")),
};
