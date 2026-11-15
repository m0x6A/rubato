import type { NewIssueRequest, NewIssueResponse } from "../../protocol.ts";
import { ApiError, request } from "../../shared/api.ts";
import { scoped } from "../../shared/workspace.ts";

export const newIssueApi = {
  /**
   * `POST /api/issues`. A failed or still-opening creation answers with a `NewIssueResponse`
   * under an error status (409, 502); that answer is returned, not thrown. A refusal with the
   * server's reason in words throws `ApiError`.
   */
  create: async (req: NewIssueRequest): Promise<NewIssueResponse> => {
    try {
      return await request<NewIssueResponse>("POST", scoped("/api/issues"), req);
    } catch (e) {
      if (e instanceof ApiError) {
        try {
          const r = JSON.parse(e.message) as NewIssueResponse;
          if (r && typeof r.state === "string") {
            return r;
          }
        } catch {
          // Plain text: the server's reason.
        }
      }
      throw e;
    }
  },
};
