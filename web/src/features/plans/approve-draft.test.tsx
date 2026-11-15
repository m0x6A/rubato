import { configure, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue, realMap } from "../../../test/fixtures.ts";
import type { ApprovalState, DraftApproveRequest, DraftSummary, IssueSummary, LaunchLog, Me, SessionLogEntry, SessionState } from "../../protocol.ts";
import { draftLaunches, newestOfKind, summaryText } from "./ApproveDraft.tsx";
import { DraftSection, WhereItFits } from "./index.ts";

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});
configure({ asyncUtilTimeout: 5000 });

const DRAFT = "d0000000-0000-4000-8000-000000000001";
const LAUNCH = "1a000000-0000-4000-8000-000000000001";
const OLD_LAUNCH = "1a000000-0000-4000-8000-000000000002";
const BRANCH = "issue/122-plan-in-rubato";
const SHA = "22e89bdc981e97d34dc7057747332a8e23cdf0e9";
const earlier = new Date(Date.now() - 3_600_000).toISOString();
const now = new Date().toISOString();
const alice: Me = { login: "alice" };
const bob: Me = { login: "bob" };

const session = (extra: Partial<SessionState> = {}): SessionState => ({
  companion_id: "c1",
  user: "alice",
  cwd: "/w/122",
  branch: BRANCH,
  launch_id: LAUNCH,
  source: "hook",
  status: "idle",
  last_seen: now,
  ...extra,
});

const draft = (extra: Partial<DraftSummary> = {}): DraftSummary => ({ draft_id: DRAFT, branch: BRANCH, owner: "alice", rev: 3, at: now, plan_sha: SHA, bundle_hash: "b3", live: true, ...extra });

const drafted = (d: Partial<DraftSummary> = {}, extra: Partial<IssueSummary> = {}): IssueSummary =>
  issue(122, "web", { draft: draft(d), sessions: [session(), session({ launch_id: OLD_LAUNCH, session_id: "s0" })], ...extra });

const entry = (kind: SessionLogEntry["kind"], text: string, at = now): SessionLogEntry => ({ session_id: "s1", prompt_id: `p-${kind}-${at}`, role: kind === "prompt" ? "user" : "assistant", kind, text, at });

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

const LOGS: Record<string, SessionLogEntry[]> = {
  [LAUNCH]: [entry("prompt", "write the plan"), entry("review", "Review: full — passed\n\nAll rows checked."), entry("summary", "Summary: we chose the tmux paste\nand hook payloads.")],
  [OLD_LAUNCH]: [entry("summary", "Summary: an older summary", earlier)],
};

/** A server with the draft, its revisions, the two launch logs and the approve route. */
function serve(approve: (req: DraftApproveRequest) => Response = (req) => json({ approval_id: "ap1", approved_by: "bob", approved_at: now, self_approved: false, plan_sha: req.plan_sha, bundle_hash: "b3", created: true })) {
  const posted: DraftApproveRequest[] = [];
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const url = String(input);
    const log = /^\/api\/launches\/([^/]+)\/log$/.exec(url);
    if (log) {
      const id = decodeURIComponent(log[1] as string);
      return json({ launch_id: id, entries: LOGS[id] ?? [] } satisfies LaunchLog);
    }
    if (url === "/api/plans/drafts/approve" && init?.method === "POST") {
      const req = JSON.parse(String(init.body)) as DraftApproveRequest;
      posted.push(req);
      return approve(req);
    }
    if (url === `/api/drafts/${DRAFT}`) {
      return json({ draft_id: DRAFT, issue: { repo: "m0x6A/rubato", number: 122 }, branch: BRANCH, owner: "alice", revisions: [1, 2, 3].map((rev) => ({ rev, at: now, plan_sha: rev === 3 ? SHA : `p${rev}`, bundle_hash: `b${rev}` })) });
    }
    if (url.startsWith(`/api/drafts/${DRAFT}/revisions/`)) {
      return json({ draft_id: DRAFT, rev: 3, at: now, plan_sha: SHA, bundle_hash: "b3", files: [{ path: "docs/plans/122.md", text: "# Plan\n" }] });
    }
    return json({ error: "unknown api path" }, 404);
  });
  return { posted, urls: () => fetch.mock.calls.map((c) => String(c[0])) };
}

const form = () => screen.findByTestId("approve-draft");
const summaryBox = () => screen.getByRole("textbox", { name: "Summary of the discussion" });

describe("approve a draft in Rubato (ADR 0054)", () => {
  it("is not offered until the cutover is set: the server would refuse it", async () => {
    serve();
    render(<DraftSection issue={drafted()} me={bob} />);
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
    expect(screen.queryByTestId("approve-draft")).not.toBeInTheDocument();
  });

  it("starts the summary from the agent's newest Summary: reply of the owner's launches on the branch, and shows its review", async () => {
    const { urls } = serve();
    render(<DraftSection issue={drafted()} me={bob} approvalsInRubato />);
    const f = await form();
    expect(within(f).getByRole("heading", { name: "Approve revision 3" })).toBeInTheDocument();
    await waitFor(() => expect(summaryBox()).toHaveValue("we chose the tmux paste\nand hook payloads."));
    expect(f).toHaveTextContent("The agent's review");
    expect(f).toHaveTextContent("Review: full — passed");
    expect(urls()).toContain(`/api/launches/${LAUNCH}/log`);
    expect(urls()).toContain(`/api/launches/${OLD_LAUNCH}/log`);
  });

  it("posts the newest revision's sha, its number and the summary as edited, and says it was approved", async () => {
    const { posted } = serve();
    render(<DraftSection issue={drafted()} me={bob} approvalsInRubato />);
    await form();
    await waitFor(() => expect(summaryBox()).not.toBeDisabled());
    fireEvent.change(summaryBox(), { target: { value: "We chose b." } });
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await screen.findByText("Approved revision 3 as you.");
    expect(posted).toEqual([{ repo: "m0x6A/rubato", number: 122, branch: BRANCH, plan_sha: SHA, rev: 3, summary: "We chose b." }]);
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
  });

  it("says when the owner approves their own agent's plan: allowed, and recorded as self", async () => {
    serve((req) => json({ approval_id: "ap1", approved_by: "alice", approved_at: now, self_approved: true, plan_sha: req.plan_sha, bundle_hash: "b3", created: true }));
    render(<DraftSection issue={drafted()} me={alice} approvalsInRubato />);
    const f = await form();
    expect(f).toHaveTextContent("recorded as self-approval");
    await waitFor(() => expect(summaryBox()).not.toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await screen.findByText("Approved revision 3 as you (self).");
  });

  it("puts a refusal in words with its code", async () => {
    serve(() => json({ reason: "not_settled" }, 409));
    render(<DraftSection issue={drafted()} me={bob} approvalsInRubato />);
    await form();
    await waitFor(() => expect(summaryBox()).not.toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    await screen.findByText("The session is still working on the draft; approve when it waits for input (not_settled)");
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("cannot be pressed while the plan links files that are not written yet", async () => {
    const { posted } = serve();
    render(<DraftSection issue={drafted({ missing: ["docs/plans/122-x/design.md"] })} me={bob} approvalsInRubato />);
    await form();
    await waitFor(() => expect(summaryBox()).not.toBeDisabled());
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByTestId("approve-why")).toHaveTextContent("The plan links files that are not written yet.");
    expect(posted).toEqual([]);
  });

  it("shows the approval instead of the form once this revision is approved in Rubato", async () => {
    serve();
    const approval: ApprovalState = { record: { author: "alice", created_at: now, issue: { repo: "m0x6A/rubato", number: 122 }, sha: SHA, source: "rubato", approval_id: "ap1", url: "" }, version: "current", current_sha: SHA };
    render(<DraftSection issue={drafted({}, { approval })} me={bob} approvalsInRubato />);
    const shown = await screen.findByTestId("draft-approved");
    expect(shown).toHaveTextContent("approved revision 3 by alice (self)");
    expect(screen.queryByTestId("approve-draft")).not.toBeInTheDocument();
  });
});

describe("Accept plan after the cutover", () => {
  const planned = issue(122, "web", { plan_sha: SHA, fit: { fits: true, reasons: [] } as NonNullable<IssueSummary["fit"]> });

  it("is offered before the cutover", () => {
    render(<WhereItFits issue={planned} map={realMap()} me={bob} onNotice={() => {}} />);
    expect(screen.getByRole("button", { name: "Accept plan" })).toBeInTheDocument();
  });

  it("gives way to approving the draft once the cutover is set", () => {
    render(<WhereItFits issue={planned} map={realMap()} me={bob} onNotice={() => {}} approvalsInRubato />);
    expect(screen.queryByRole("button", { name: "Accept plan" })).not.toBeInTheDocument();
    expect(screen.getByTestId("approve-in-rubato")).toHaveTextContent("Plans are approved in Rubato on this server");
  });
});

describe("where the summary comes from", () => {
  it("strips the Summary: label and keeps the rest", () => {
    expect(summaryText(entry("summary", "  Summary: one\ntwo "))).toBe("one\ntwo");
  });

  it("takes the newest entry of a kind", () => {
    const a = entry("summary", "Summary: a", earlier);
    const b = entry("summary", "Summary: b", now);
    expect(newestOfKind([b, a, entry("reply", "x")], "summary")).toBe(b);
    expect(newestOfKind([a], "review")).toBeNull();
  });

  it("reads only the owner's launched sessions on the draft's branch", () => {
    const i = drafted({}, { sessions: [session(), session({ user: "bob", launch_id: "other" }), session({ branch: "issue/122-else", launch_id: "elsewhere" }), session({ launch_id: null }), session()] });
    expect(draftLaunches(i, draft())).toEqual([LAUNCH]);
  });
});
