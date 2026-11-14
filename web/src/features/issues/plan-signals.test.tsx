import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { ApprovalState, ReviewRecord } from "../../protocol.ts";
import { findingText } from "../findings/index.ts";
import { IssuePanel } from "./IssuePanel.tsx";
import { SignalBadges } from "./PlanSignals.tsx";

const ref = { repo: "m0x6A/rubato", number: 71 };
const now = new Date().toISOString();

const review: ReviewRecord = { issue: ref, comment_id: 1, author: "m0x6A", track: "full", outcome: "passed after fixes", created_at: now, url: "https://github.com/m0x6A/rubato/issues/71#c1" };

function approval(version: ApprovalState["version"], branch?: string): ApprovalState {
  return {
    record: { issue: ref, comment_id: 2, author: "teammate", sha: "f3c5e84", ...(branch ? { branch } : {}), created_at: now, url: "https://github.com/m0x6A/rubato/issues/71#c2" },
    version,
    current_sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9",
  };
}

function show(extra: Parameters<typeof issue>[2], selected: boolean) {
  const ov = overview({ issues: [issue(71, "rubato-server", extra)] });
  render(<IssuePanel overview={ov} me={{ login: "m0x6A" }} selected={selected ? (ov.issues[0] ?? null) : null} selection={null} onSelect={() => {}} onClearSelection={() => {}} onLaunch={() => {}} onPin={() => {}} onUnpin={() => {}} />);
}

describe("review and approval on the issue (ADR 0027)", () => {
  it("shows a member's review and a current approval on the detail", () => {
    show({ review, approval: approval("current") }, true);
    const box = screen.getByTestId("plan-signals");
    expect(within(box).getByTestId("plan-review")).toHaveTextContent("reviewed (full, passed after fixes)");
    expect(within(box).getByTestId("plan-approval")).toHaveTextContent("approved by teammate current version");
    expect(box).toHaveTextContent("at least every 5 minutes");
  });

  it("says an approval names an earlier version, and on which branch", () => {
    show({ approval: approval("earlier", "issue/71-plan") }, true);
    const line = screen.getByTestId("plan-approval");
    expect(line).toHaveTextContent("approved an earlier version");
    expect(line).toHaveTextContent("the plan changed since");
    expect(line).toHaveTextContent("on issue/71-plan");
    expect(screen.getByTestId("plan-review")).toHaveTextContent("not reviewed");
  });

  it("tells how to approve when nobody has", () => {
    show({ review }, true);
    expect(screen.getByTestId("plan-approval")).toHaveTextContent("Approved: plan <sha>");
  });

  it("badges the row: reviewed, approved or approval stale", () => {
    show({ review, approval: approval("current") }, false);
    const row = screen.getByTestId("issue-row-71");
    expect(within(row).getByText("reviewed")).toBeInTheDocument();
    expect(within(row).getByText("approved")).toBeInTheDocument();
  });

  it("badges a stale approval differently from a current one", () => {
    show({ approval: approval("earlier") }, false);
    const row = screen.getByTestId("issue-row-71");
    expect(within(row).getByText("approval stale")).toHaveClass("approval-stale");
    expect(within(row).queryByText("approved")).toBeNull();
  });

  it("says an approval of a plan that does not fit is not counted (ADR 0029)", () => {
    show({ approval: approval("does_not_fit") }, true);
    const line = screen.getByTestId("plan-approval");
    expect(line).toHaveTextContent("approved — doesn't fit, not counted");
    expect(line).toHaveTextContent("the plan does not fit the map");
    expect(within(line).getByText("approved — doesn't fit, not counted")).toHaveClass("approval-stale");
    expect(findingText({ kind: "approved_without_fit", issue: ref })).toContain("does not count");
    expect(findingText({ kind: "unattributed_session", companion: "00000000-0000-0000-0000-000000000000", branch: "main", repo: "m0x6A/rubato" })).toBe("a session in m0x6A/rubato on branch main is not tied to an issue; name its branch issue/<n>-…");
  });

  it("explains the finding", () => {
    expect(findingText({ kind: "approval_stale", issue: ref, approved_sha: "c438e006b75b", current_sha: "f3c5e84f2bf8" })).toBe("m0x6A/rubato#71: the plan changed after it was approved (approved c438e00, now f3c5e84)");
  });
});

describe("badges across workspaces (issue #89, ADR 0045)", () => {
  it("says where else the issue is pinned and where its plan counts", () => {
    render(<SignalBadges issue={{ ...issue(5, "auth-api"), pinned_in: ["Shop"], fits_in: ["Shop"] }} here="Admin" />);
    expect(screen.getByText("pinned in Shop")).toBeInTheDocument();
    expect(screen.getByText("counts in Shop")).toBeInTheDocument();
  });

  it("says nothing extra when the plan fits here, or with one workspace", () => {
    const { container } = render(<SignalBadges issue={{ ...issue(5, "auth-api"), fits_in: ["Admin", "Shop"] }} here="Admin" />);
    expect(container).toBeEmptyDOMElement();
    const single = render(<SignalBadges issue={issue(6, "auth-api")} />);
    expect(single.container).toBeEmptyDOMElement();
  });
});
