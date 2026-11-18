import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { DraftSummary, IssueRef, IssueSummary, Me, Overview, Plan } from "../../protocol.ts";
import { PlansBoard } from "./index.ts";

const ref = (n: number) => ({ repo: "m0x6A/rubato", number: n });
const now = new Date().toISOString();
const me: Me = { login: "m0x6A", can_act_on_github: true };

const plan = (n: number): Plan => ({
  issue: ref(n),
  path: `docs/plans/${n}.md`,
  git_ref: `issue/${n}-x`,
  affected_parts: [{ node: "web", status: "affected", change: "the board", seams: ["web-server"] }],
  o_rows: [{ seam: "web-server", signal: "s", panel: "p", owner: "rubato-server" }],
});
const planned = (n: number, extra: Partial<IssueSummary> = {}) => issue(n, "web", { title: `plan ${n}`, plan: plan(n), plan_sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9", fit: { fits: true }, ...extra });
const approved = (n: number) => ({ record: { issue: ref(n), comment_id: 1, author: "m0x6A", sha: "f3c5e84", created_at: now, url: "u" }, version: "current" as const });
const draft = (n: number): DraftSummary => ({ draft_id: `00000000-0000-0000-0000-00000000000${n}`, rev: 2, at: now, branch: `issue/${n}-x`, bundle_hash: "b", plan_sha: "p", owner: "alice", live: false, fits: true });

/** Proposed #1, a draft-only #2, open #3, and two finished: #4 closed on GitHub, #5 merged. */
const board = () =>
  overview({
    issues: [
      planned(1),
      issue(2, "web", { title: "only a draft", draft: draft(2) }),
      planned(3, { approval: approved(3) }),
      planned(4, { closed: true, finished: true }),
      planned(5, { progress: { issue: ref(5), state: "done", last_seen: now }, approval: approved(5) }),
    ],
  });

/** The board in a shell that holds the selection, as App does. */
function Shell({ ov, onOpenIssue = () => {} }: { ov: Overview; onOpenIssue?: (i: IssueRef) => void }) {
  const [sel, setSel] = useState<IssueRef | null>(null);
  const selected = sel ? (ov.issues.find((i) => i.issue.number === sel.number) ?? null) : null;
  return <PlansBoard overview={ov} me={me} selected={selected} onSelect={setSel} onOpenIssue={onOpenIssue} onNotice={() => {}} onResume={() => {}} />;
}

const rowsIn = (testid: string) =>
  within(screen.getByTestId(testid))
    .queryAllByTestId(/^plan-row-\d+$/)
    .map((el) => el.getAttribute("data-testid"));

afterEach(() => vi.restoreAllMocks());

describe("the plans board (#265)", () => {
  it("shows Proposed and Open, a draft among the proposed, and no finished plan", () => {
    render(<Shell ov={board()} />);
    expect(rowsIn("plans-proposed")).toEqual(["plan-row-1", "plan-row-2"]);
    expect(rowsIn("plans-open")).toEqual(["plan-row-3"]);
    expect(screen.queryByTestId("plans-done")).toBeNull();
    expect(screen.queryByTestId("plan-row-4")).toBeNull();
    expect(screen.queryByTestId("plan-row-5")).toBeNull();
    expect(screen.getByTestId("plans-view-open")).toHaveTextContent("Open (3)");
    expect(screen.getByTestId("plans-view-done")).toHaveTextContent("Done (2)");
    // The draft's row says whose it is; it is approved on the draft, so the row has no Accept.
    const draftRow = screen.getByTestId("plan-row-2");
    expect(draftRow).toHaveTextContent("draft · revision 2 · alice");
    expect(draftRow).toHaveTextContent("fits");
    expect(within(draftRow).queryByTestId("accept-plan")).toBeNull();
    expect(within(screen.getByTestId("plan-row-1")).getByTestId("accept-plan")).toBeInTheDocument();
  });

  it("Done lists the finished plans, marked merged or closed, with nothing to accept", () => {
    render(<Shell ov={board()} />);
    fireEvent.click(screen.getByTestId("plans-view-done"));
    expect(rowsIn("plans-done").sort()).toEqual(["plan-row-4", "plan-row-5"]);
    expect(screen.queryByTestId("plans-proposed")).toBeNull();
    expect(within(screen.getByTestId("plan-row-4")).getByText("closed")).toBeInTheDocument();
    expect(within(screen.getByTestId("plan-row-5")).getByText("merged")).toBeInTheDocument();
    expect(screen.queryByTestId("accept-plan")).toBeNull();
  });

  it("a plan opens in the Plans tab, goes back to the board, and can open its issue", () => {
    const onOpenIssue = vi.fn();
    render(<Shell ov={board()} onOpenIssue={onOpenIssue} />);
    fireEvent.click(within(screen.getByTestId("plan-row-1")).getByRole("button", { name: "m0x6A/rubato#1" }));
    const detail = screen.getByTestId("plan-detail");
    expect(within(detail).getByRole("heading", { level: 2, name: "m0x6A/rubato#1" })).toHaveFocus();
    expect(within(detail).getByTestId("where-it-fits")).toBeInTheDocument();
    expect(within(detail).getByTestId("accept-plan")).toBeInTheDocument();
    // No draft in Rubato: the text is on GitHub, on the plan's branch.
    expect(within(detail).getByRole("link", { name: "Read docs/plans/1.md on GitHub" })).toHaveAttribute("href", "https://github.com/m0x6A/rubato/blob/issue/1-x/docs/plans/1.md");
    expect(screen.queryByTestId("plans-panel")).toBeNull();
    fireEvent.click(within(detail).getByTestId("plan-open-issue"));
    expect(onOpenIssue).toHaveBeenCalledWith(ref(1));
    fireEvent.click(within(detail).getByRole("button", { name: "← all plans" }));
    expect(screen.getByTestId("plans-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("plan-detail")).toBeNull();
  });

  it("a finished plan's detail is read-only: no Accept, Import, Approve or Resume", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(JSON.stringify({ draft_id: draft(4).draft_id, branch: "issue/4-x", issue: ref(4), owner: "m0x6A", revisions: [] }), { status: 200 }));
    const mine = { ...draft(4), owner: "m0x6A" };
    const ov = overview({ issues: [planned(4, { closed: true, finished: true, draft: mine, outside_rubato: true }), planned(6, { draft: { ...mine, branch: "issue/6-x" } })], comment_approvals_until: now });
    const { unmount } = render(<PlansBoard overview={ov} me={me} selected={ov.issues[0]!} onSelect={() => {}} onOpenIssue={() => {}} onNotice={() => {}} onResume={() => {}} />);
    const detail = screen.getByTestId("plan-detail");
    expect(within(detail).getByTestId("plan-finished-note")).toBeInTheDocument();
    expect(within(detail).getByTestId("draft")).toBeInTheDocument();
    for (const id of ["accept-plan", "import-plan", "approve-draft", "resume-draft"]) {
      expect(within(detail).queryByTestId(id), id).toBeNull();
    }
    unmount();
    // Must fail on purpose if these are gone for every plan: the open one's draft can be resumed.
    render(<PlansBoard overview={ov} me={me} selected={ov.issues[1]!} onSelect={() => {}} onOpenIssue={() => {}} onNotice={() => {}} onResume={() => {}} />);
    expect(await screen.findByTestId("resume-draft")).toBeInTheDocument();
    expect(screen.queryByTestId("plan-finished-note")).toBeNull();
  });

  it("an issue with no plan selected elsewhere leaves the board in place", () => {
    const ov = overview({ issues: [issue(9, "web"), planned(1)] });
    render(<PlansBoard overview={ov} me={me} selected={ov.issues[0]!} onSelect={() => {}} onOpenIssue={() => {}} onNotice={() => {}} />);
    expect(screen.getByTestId("plans-panel")).toBeInTheDocument();
  });
});
