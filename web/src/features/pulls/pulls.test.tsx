import { configure, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MergeRequest, OpenPull, PullDetail } from "../../protocol.ts";
import { WHY_NOT } from "./api.ts";
import { MergeButton, PullList, PullView } from "./index.ts";

afterEach(() => vi.restoreAllMocks());
configure({ asyncUtilTimeout: 5000 });

const now = new Date().toISOString();
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { "content-type": "application/json" } });

const pull = (n: number, extra: Partial<OpenPull> = {}): OpenPull => ({ repo: "m0x6A/rubato", number: n, title: `Pull ${n}`, author: "ann", branch: `issue/${n}-x`, base: "main", draft: false, head_sha: `h${n}`, updated_at: now, url: `https://github.com/m0x6A/rubato/pull/${n}`, ...extra });

const detail = (extra: Partial<PullDetail> = {}, merge: Partial<PullDetail["merge"]> = {}): PullDetail => ({
  repo: "m0x6A/rubato",
  number: 5,
  title: "Pulls in Rubato",
  author: "ann",
  body: "<b>not markup</b>\nsecond line",
  state: "open",
  draft: false,
  branch: "issue/82-x",
  base: "main",
  head_sha: "h5",
  mergeable: true,
  checks: "green",
  check_runs: [{ name: "ci", status: "completed", conclusion: "success", url: "https://ci/1" }],
  reviews: [{ author: "m0x6A", state: "APPROVED", submitted_at: now }],
  outside_reviews: 2,
  files: { count: 3, additions: 10, deletions: 4, top: [{ path: "a.rs", additions: 8, deletions: 1 }], truncated: false },
  url: "https://github.com/m0x6A/rubato/pull/5",
  issue: { repo: "m0x6A/rubato", number: 82 },
  read_at: now,
  merge: { methods: ["merge", "squash"], default_method: "squash", can_merge: true, plan_gate: { kind: "no_plan" }, stacked_above: 0, ...merge },
  ...extra,
});

describe("PullList (issue #82)", () => {
  it("lists each pull request with its checks and opens the one chosen", () => {
    const opened: number[] = [];
    render(<PullList pulls={[pull(1, { checks: "green" }), pull(2, { checks: "red", draft: true }), pull(3)]} onOpen={(p) => opened.push(p.number)} truncated />);
    expect(screen.getByTestId("pull-row-1")).toHaveTextContent("checks green");
    expect(screen.getByTestId("pull-row-2")).toHaveTextContent("draft");
    expect(screen.getByTestId("pull-row-2")).toHaveTextContent("checks red");
    expect(screen.getByTestId("pull-row-3")).toHaveTextContent("checks not read yet");
    expect(screen.getByText("Only the first 100 are listed.")).toBeInTheDocument();
    fireEvent.click(within(screen.getByTestId("pull-row-2")).getByRole("button"));
    expect(opened).toEqual([2]);
  });

  it("says so when there are none", () => {
    render(<PullList pulls={[]} onOpen={() => {}} />);
    expect(screen.getByText("No open pull requests from this repo's branches.")).toBeInTheDocument();
  });
});

describe("MergeButton (ADR 0033)", () => {
  it("offers only the allowed methods, squash by default, and merges at the head shown", async () => {
    const sent: MergeRequest[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(async (_input, init) => {
      sent.push(JSON.parse(String(init?.body)) as MergeRequest);
      return json({ state: "merged", login: "m0x6A", method: "merge", merge_sha: "m1" });
    });
    render(<MergeButton detail={detail()} />);
    const select = screen.getByRole("combobox");
    expect([...select.querySelectorAll("option")].map((o) => o.getAttribute("value"))).toEqual(["merge", "squash"]);
    expect(select).toHaveValue("squash");
    fireEvent.change(select, { target: { value: "merge" } });
    fireEvent.click(screen.getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Merged by m0x6A."));
    expect(sent).toEqual([{ repo: "m0x6A/rubato", number: 5, head_sha: "h5", method: "merge" }]);
  });

  it("is off with the reason in words for every reason the server gives", () => {
    const reasons = ["not_open", "fork", "draft", "checks_not_green", "not_mergeable", "plan_not_approved", "needs_github_authorization", "no_push_permission"];
    for (const why of reasons) {
      const { unmount } = render(<MergeButton detail={detail({}, { can_merge: false, why_not: why, methods: [] })} />);
      expect(screen.getByRole("button", { name: "Merge" })).toBeDisabled();
      expect(screen.getByTestId("merge-why-not")).toHaveTextContent(WHY_NOT[why]!);
      unmount();
    }
    render(<MergeButton detail={detail({}, { can_merge: false, why_not: "plan_not_approved", plan_gate: { kind: "not_approved", why: "changed_at_head" } })} />);
    expect(screen.getByTestId("merge-why-not")).toHaveTextContent("the plan at this head is not the version approved");
  });

  it("warns that a squash makes the pull requests based on this branch conflict", () => {
    render(<MergeButton detail={detail({}, { stacked_above: 2 })} />);
    expect(screen.getByTestId("merge-stacked")).toHaveTextContent("2 open pull requests are based on this branch");
  });

  it("shows GitHub's own refusal", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => json({ reason: "github_refused", status: 405, message: "Base branch was modified" }, 502));
    render(<MergeButton detail={detail()} />);
    fireEvent.click(screen.getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("GitHub refused the merge. Base branch was modified"));
  });
});

describe("PullView (design 82 §5)", () => {
  it("shows body as text, checks, team reviews, the outside count and the files", async () => {
    const urls: string[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
      urls.push(String(input));
      return json(detail());
    });
    const opened: number[] = [];
    render(<PullView repo="m0x6A/rubato" number={5} onOpenIssue={(i) => opened.push(i.number)} />);
    const view = await screen.findByTestId("pull-body");
    expect(view.textContent).toBe("<b>not markup</b>\nsecond line");
    expect(screen.getByText("ci")).toHaveAttribute("href", "https://ci/1");
    expect(screen.getByText("2 reviews from outside the team, not shown.")).toBeInTheDocument();
    expect(screen.getByTestId("pull-files")).toHaveTextContent("3 files · +10 −4");
    expect(screen.getByRole("link", { name: "View diff on GitHub" })).toHaveAttribute("href", "https://github.com/m0x6A/rubato/pull/5/files");
    fireEvent.click(screen.getByRole("button", { name: "#82" }));
    expect(opened).toEqual([82]);
    expect(urls).toEqual(["/api/pulls/m0x6A/rubato/5"]);
  });

  it("says why it could not read, and tries again on request", async () => {
    let n = 0;
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => (n++ === 0 ? json({ reason: "unknown_repo" }, 404) : json(detail())));
    render(<PullView repo="m0x6A/rubato" number={5} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("This repo is not in the map");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByTestId("pull-body")).toBeInTheDocument();
  });
});
