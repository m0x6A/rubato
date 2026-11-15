import { describe, expect, it } from "vitest";
import type { LaunchResponse } from "../../protocol.ts";
import { ApiError } from "../../shared/api.ts";
import { launchFailureText, launchNotice } from "./notice.ts";

const issue = { repo: "m0x6A/rubato", number: 3 };
function response(extra: Partial<LaunchResponse["command"]> = {}): LaunchResponse {
  return {
    companion_id: "c1",
    command: {
      id: "1a2b3c4d-0000-4000-8000-000000000000",
      user: "m0x6A",
      issue,
      repo_url: "https://github.com/m0x6A/rubato.git",
      branch: "issue/3-first-title",
      base_ref: "origin/main",
      node_ids: [],
      spend_cap_usd: 4.5,
      attempt_limit: 7,
      attempt: 2,
      session_id: "s",
      prompt: "p",
      otel_resource: {},
      ...extra,
    },
  };
}

describe("the launch banner (ADR 0053)", () => {
  it("says when the draft travels with the launch", () => {
    expect(launchNotice(response(), issue)).toBe("Launch 1a2b3c4d sent to your companion for m0x6A/rubato#3 on issue/3-first-title.");
    expect(launchNotice(response({ draft: { draft_id: "d", rev: 4, files: [] } }), issue)).toBe(
      "Launch 1a2b3c4d sent to your companion for m0x6A/rubato#3 on issue/3-first-title, with draft revision 4 to write into a new worktree.",
    );
  });

  it("puts pass_branch in words, naming the branches", () => {
    const e = new ApiError(409, "pass_branch: you own drafts of m0x6A/rubato#3 on issue/3-one, issue/3-two; pass branch");
    expect(launchFailureText(e)).toBe("you own drafts of it on several branches (issue/3-one, issue/3-two): resume one from its draft (pass_branch)");
  });
});
