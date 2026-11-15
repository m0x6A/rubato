import type { IssueRef, LaunchResponse } from "../../protocol.ts";
import { ApiError, failureText } from "../../shared/api.ts";
import { issueKey } from "../../shared/format.ts";

/** The banner shown after the server accepted a launch for the caller's own companion. */
export function launchNotice(r: LaunchResponse, issue: IssueRef): string {
  const restored = r.command.draft ? `, with draft revision ${r.command.draft.rev} to write into a new worktree` : "";
  return `Launch ${r.command.id.slice(0, 8)} sent to your companion for ${issueKey(issue)} on ${r.command.branch}${restored}.`;
}

/** Why a launch was refused, in words: several drafts of yours need one named (ADR 0053). */
export function launchFailureText(e: unknown): string {
  if (e instanceof ApiError && e.status === 409 && e.message.startsWith("pass_branch:")) {
    const branches = /on (.*); pass branch/.exec(e.message)?.[1];
    return `you own drafts of it on several branches${branches ? ` (${branches})` : ""}: resume one from its draft (pass_branch)`;
  }
  return failureText(e);
}
