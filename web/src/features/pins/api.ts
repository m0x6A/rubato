import type { PinRequest } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const pinsApi = {
  addPin: (req: PinRequest) => request<void>("POST", "/api/pins", req),
  removePin: (req: PinRequest) => request<void>("DELETE", "/api/pins", req),
};
