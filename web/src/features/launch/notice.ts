import type { IssueRef, LaunchResponse } from "../../protocol.ts";
import { issueKey } from "../../shared/format.ts";

/** The banner shown after the server accepted a launch for the caller's own companion. */
export function launchNotice(r: LaunchResponse, issue: IssueRef): string {
  return `Launch ${r.command.id.slice(0, 8)} sent to your companion for ${issueKey(issue)} on ${r.command.branch}.`;
}
