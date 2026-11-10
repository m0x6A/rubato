import type { PairApproveRequest, PairingInfo } from "../../protocol.ts";
import { request } from "../../shared/api.ts";

export const companionsApi = {
  approvePairing: (req: PairApproveRequest) => request<PairingInfo>("POST", "/api/pair/approve", req),
  revokePairing: (companionId: string) => request<void>("DELETE", `/api/pairings/${companionId}`),
};
