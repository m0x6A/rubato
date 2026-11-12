import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { ApprovalState, IssueRef } from "../../protocol.ts";
import { approvalCounts, buildGraph } from "./graph.ts";
import { PartCard } from "./PartCard.tsx";

function approval(ref: IssueRef, version: ApprovalState["version"]): ApprovalState {
  return { record: { issue: ref, comment_id: 1, author: "m0x6A", sha: "f3c5e84", created_at: new Date().toISOString(), url: "u" }, version };
}

describe("approvals on a part (ADR 0027)", () => {
  const ref = (n: number) => ({ repo: "m0x6A/rubato", number: n });
  const ov = overview({
    issues: [
      issue(1, "rubato-server", { approval: approval(ref(1), "current") }),
      issue(2, "rubato-server", { approval: approval(ref(2), "earlier") }),
      issue(3, "rubato-server", { approval: approval(ref(3), "not_named") }),
      issue(4, "rubato-server"),
    ],
    findings: [{ kind: "approval_stale", issue: ref(2), approved_sha: "c438e00", current_sha: "f3c5e84" }],
  });
  const node = buildGraph(ov).nodes.find((n) => n.id === "rubato-server");

  it("counts current and stale approvals, nothing else", () => {
    expect(approvalCounts(node?.issues ?? [])).toEqual({ approved: 1, stale: 1 });
  });

  it("shows both on the card, and the stale one as a finding on the part", () => {
    if (!node) throw new Error("no rubato-server part");
    render(<PartCard node={node} selected={false} />);
    const card = screen.getByTestId("node-rubato-server");
    expect(card).toHaveTextContent("1 approved");
    expect(card).toHaveTextContent("1 approval stale");
    expect(card).toHaveTextContent("⚠ 1 finding");
  });
});
