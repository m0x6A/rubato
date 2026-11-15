import type { PinRequest } from "../../protocol.ts";
import { request } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

export const pinsApi = {
  addPin: (req: PinRequest) => request<void>("POST", scoped("/api/pins"), req),
  removePin: (req: PinRequest) => request<void>("DELETE", scoped("/api/pins"), req),
};
