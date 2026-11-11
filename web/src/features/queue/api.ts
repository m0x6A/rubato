import type { IssueRef, QueueOrder, QueueRequest } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const queueApi = {
  /** Queue an issue on one of your own companions. */
  add: (req: QueueRequest) => request<void>("POST", "/api/queue", req),
  /** The full order of one lane. Refused when the lane changed meanwhile. */
  reorder: (companion_id: string, issues: IssueRef[]) => request<void>("PUT", "/api/queue", { companion_id, issues } satisfies QueueOrder),
  remove: (req: QueueRequest) => request<void>("DELETE", "/api/queue", req),
};
