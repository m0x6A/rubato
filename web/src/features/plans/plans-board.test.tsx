import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { DraftSummary, IssueRef, IssueSummary, Me, Overview, Plan } from "../../protocol.ts";
import { PlanFileNote, PlansBoard, WhereItFits } from "./index.ts";

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

/** The board as the shell mounts it: a row hands its issue up, and the shell opens the issue's plan. */
function Shell({ ov, onOpen = () => {} }: { ov: Overview; onOpen?: (i: IssueRef) => void }) {
  const [done, setDone] = useState(false);
  return <PlansBoard overview={ov} done={done} onDone={setDone} onOpen={onOpen} />;
}

const rowsIn = (testid: string) =>
  within(screen.getByTestId(testid))
    .queryAllByTestId(/^plan-row-\d+$/)
    .map((el) => el.getAttribute("data-testid"));

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

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
    // The draft's row says whose it is. No row accepts: a plan is accepted where it is read (issue #357).
    const draftRow = screen.getByTestId("plan-row-2");
    expect(draftRow).toHaveTextContent("draft · revision 2 · alice");
    expect(screen.queryByTestId("accept-plan")).toBeNull();
    expect(screen.queryByRole("button", { name: "Accept plan" })).toBeNull();
  });

  it("a row badges only what calls for something: a plan that does not fit, an approval that went stale", () => {
    const stale = { ...approved(7), version: "earlier" as const };
    render(<Shell ov={overview({ issues: [planned(1, { review: { issue: ref(1), comment_id: 2, author: "m0x6A", created_at: now, url: "u", track: "small", outcome: "passed" } as never }), planned(6, { fit: { fits: false, reasons: [] } as never }), planned(7, { approval: stale }), planned(3, { approval: approved(3) })] })} />);
    const badges = (n: number) => [...screen.getByTestId(`plan-row-${n}`).querySelectorAll(".row-end .badge")].map((b) => b.textContent);
    expect(badges(1)).toEqual([]);
    expect(badges(3)).toEqual([]);
    expect(badges(6)).toEqual(["does not fit"]);
    expect(badges(7)).toHaveLength(1);
    expect(screen.getByTestId("plan-row-7").querySelector(".badge")).toHaveClass("approval-stale");
    // The headings carry the counts; nothing repeats them above the columns.
    expect(document.querySelector(".big-counts")).toBeNull();
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

  it("a row opens its plan: the board hands the issue up", () => {
    const onOpen = vi.fn();
    render(<Shell ov={board()} onOpen={onOpen} />);
    fireEvent.click(within(screen.getByTestId("plan-row-1")).getByRole("button", { name: "m0x6A/rubato#1" }));
    expect(onOpen).toHaveBeenCalledWith(ref(1));
    // The board itself shows no plan: the issue's Plan part does.
    expect(screen.getByTestId("plans-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("where-it-fits")).toBeNull();
  });

  it("with no draft in Rubato, the plan's text is on GitHub: on its branch while open, on the default branch once merged", () => {
    const { unmount } = render(<PlanFileNote issue={planned(1)} />);
    expect(screen.getByRole("link", { name: "Read docs/plans/1.md on GitHub" })).toHaveAttribute("href", "https://github.com/m0x6A/rubato/blob/issue/1-x/docs/plans/1.md");
    unmount();
    const merged = render(<PlanFileNote issue={planned(5, { progress: { issue: ref(5), state: "done", last_seen: now } })} />);
    expect(screen.getByRole("link", { name: "Read docs/plans/5.md on GitHub" })).toHaveAttribute("href", "https://github.com/m0x6A/rubato/blob/HEAD/docs/plans/5.md");
    merged.unmount();
    // A draft in Rubato is the text: nothing to point at.
    const { container } = render(<PlanFileNote issue={planned(2, { draft: draft(2) })} />);
    expect(container).toBeEmptyDOMElement();
  });
});

describe("Accept plan says what it did, where it was pressed (#283)", () => {
  it("reads Accepting… while it posts, and the plan keeps the sentence once the approval counts", async () => {
    let answer: (r: Response) => void = () => {};
    const posted: unknown[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: string, init?: RequestInit) => {
        posted.push(JSON.parse(String(init?.body)));
        return new Promise<Response>((resolve) => {
          answer = resolve;
        });
      }),
    );
    const map = overview().map.map;
    const { rerender } = render(<WhereItFits issue={planned(1)} map={map} me={me} onNotice={() => {}} />);
    const row = () => screen.getByTestId("where-it-fits");
    fireEvent.click(within(row()).getByRole("button", { name: "Accept plan" }));
    const busy = within(row()).getByRole("button", { name: "Accepting…" });
    expect(busy).toBeDisabled();
    expect(busy).toHaveAttribute("aria-busy", "true");
    expect(posted).toEqual([{ issue: ref(1), sha: "f3c5e84f2bf82c2a409ad90e83f3236012b1dfb9" }]);

    answer(new Response(JSON.stringify({ state: "posted", body: "Approved: plan" }), { status: 200 }));
    await waitFor(() => expect(within(row()).getByRole("status")).toHaveTextContent('Accepted: posted "Approved: plan f3c5e84f2b" as you.'));

    // The overview now counts the approval: the button is gone, and the plan still says what
    // happened. Must fail on purpose if the sentence goes with the button.
    rerender(<WhereItFits issue={planned(1, { approval: approved(1) })} map={map} me={me} onNotice={() => {}} />);
    expect(within(row()).queryByRole("button", { name: "Accept plan" })).toBeNull();
    expect(within(row()).getByTestId("plan-accepted")).toHaveTextContent("Accepted: posted");
    console.log("checked 1 accept: busy label, sentence before and after the approval counted");
  });

  it("a refusal stays under the button, which is offered again", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("GitHub is down", { status: 502 })));
    render(<WhereItFits issue={planned(1)} map={overview().map.map} me={me} onNotice={() => {}} />);
    const row = screen.getByTestId("where-it-fits");
    fireEvent.click(within(row).getByRole("button", { name: "Accept plan" }));
    await waitFor(() => expect(within(row).getByRole("status")).toHaveTextContent("The plan was not accepted"));
    expect(within(row).getByRole("button", { name: "Accept plan" })).toBeEnabled();
  });
});
