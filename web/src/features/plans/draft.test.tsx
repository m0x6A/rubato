import { act, configure, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue } from "../../../test/fixtures.ts";
import type { DraftFile, DraftRevisionInfo, DraftSummary, IssueSummary, Me } from "../../protocol.ts";
import { emitWebEvent } from "../live/index.ts";
import { DraftSection } from "./index.ts";
import { readSeen, writeSeen } from "./seen.ts";

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});
// As in the chat tests: a first render can pass 1 s while every file runs at once.
configure({ asyncUtilTimeout: 5000 });

const DRAFT = "d0000000-0000-4000-8000-000000000001";
const OTHER = "d0000000-0000-4000-8000-000000000002";
const VIEW = `/api/drafts/${DRAFT}`;
const PLAN = "docs/plans/122.md";
const BRANCH = "issue/122-plan-in-rubato";
const now = new Date().toISOString();
const colleague: Me = { login: "bob" };

const FILES: Record<number, DraftFile[]> = {
  1: [
    { path: PLAN, text: "# Plan\nold line\nkept\n" },
    { path: "docs/plans/122-x/design.md", text: "gone\n" },
  ],
  2: [{ path: PLAN, text: "# Plan\nmiddle\nkept\n" }],
  3: [
    { path: PLAN, text: "# Plan\nnew line\nkept\n" },
    { path: "docs/plans/122-x/a.md", text: "a\n" },
    { path: "docs/decisions/0053-x.md", text: "adr\n" },
  ],
};

const info = (rev: number, extra: Partial<DraftRevisionInfo> = {}): DraftRevisionInfo => ({ rev, at: now, plan_sha: `p${rev}`, bundle_hash: `h${rev}`, ...extra });

function summary(extra: Partial<DraftSummary> = {}): DraftSummary {
  return { draft_id: DRAFT, branch: BRANCH, owner: "alice", rev: 3, at: now, plan_sha: "p3", bundle_hash: "h3", live: true, ...extra };
}

const drafted = (extra: Partial<DraftSummary> = {}): IssueSummary => issue(122, "web", { draft: summary(extra) });

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

/** A server holding the draft with `revs()` and the files of revisions 1–3. */
function serve(revs: () => number[] = () => [1, 2, 3], opts: { view?: Response } = {}) {
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
    const url = String(input);
    const m = /^\/api\/drafts\/([^/]+)\/revisions\/(\d+)$/.exec(url);
    if (m && m[1] === DRAFT) {
      const rev = Number(m[2]);
      const files = FILES[rev];
      return files ? json({ draft_id: DRAFT, ...info(rev), files }) : json({ reason: "unknown_draft" }, 404);
    }
    if (url === VIEW) {
      return opts.view ?? json({ draft_id: DRAFT, issue: { repo: "m0x6A/rubato", number: 122 }, branch: BRANCH, owner: "alice", revisions: revs().map((r) => info(r)) });
    }
    return json({ reason: "unknown_draft" }, 404);
  });
  const urls = () => fetch.mock.calls.map((c) => String(c[0]));
  return { urls, viewReads: () => urls().filter((u) => u === VIEW).length, revisionReads: (rev: number) => urls().filter((u) => u === `${VIEW}/revisions/${rev}`).length };
}

const revisionButtons = () => screen.getAllByRole("button", { name: /^Revision \d+/ });
const filesOf = (rev: number) => screen.getByRole("region", { name: `Files of revision ${rev}` });
const paths = (el: HTMLElement) => [...el.querySelectorAll(".draft-file > summary code")].map((c) => c.textContent);

describe("the draft on the issue (ADR 0053)", () => {
  it("says who owns it, which revision and when, and that the owner's session is working on it", async () => {
    serve();
    const { unmount } = render(<DraftSection issue={drafted()} me={colleague} />);
    const section = screen.getByTestId("draft");
    expect(within(section).getByRole("heading", { name: "Draft · revision 3" })).toBeInTheDocument();
    expect(section).toHaveTextContent("owned by alice");
    expect(section).toHaveTextContent(`on ${BRANCH}`);
    expect(section).toHaveTextContent("being worked on in alice's session");
    expect(section.querySelector("time")).toHaveAttribute("datetime", now);
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
    unmount();
    render(<DraftSection issue={drafted()} me={{ login: "Alice" }} />);
    expect(screen.getByTestId("draft")).toHaveTextContent("being worked on in your session");
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
  });

  it("is parked when no session of the owner works on it", async () => {
    serve();
    render(<DraftSection issue={drafted({ live: false })} me={colleague} />);
    expect(screen.getByTestId("draft")).toHaveTextContent("parked");
    expect(screen.getByTestId("draft")).not.toHaveTextContent("being worked on");
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
  });

  it("names the files the plan links that are not written yet, and why the plan does not parse", async () => {
    serve();
    render(<DraftSection issue={drafted({ missing: ["docs/plans/122-x/design.md", "docs/decisions/0060-y.md"], parse_error: "no affected parts table" })} me={colleague} />);
    const missing = screen.getByRole("list", { name: "The plan links these, which are not written yet" });
    expect(within(missing).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["docs/plans/122-x/design.md", "docs/decisions/0060-y.md"]);
    expect(screen.getByText("The plan does not parse: no affected parts table")).toBeInTheDocument();
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
  });
});

describe("revisions and their files", () => {
  it("lists the revisions newest first from the team route, and shows the newest one's files: the plan first, then by path", async () => {
    const { urls } = serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    await waitFor(() => expect(revisionButtons()).toHaveLength(3));
    expect(revisionButtons().map((b) => b.textContent)).toEqual(["Revision 3", "Revision 2", "Revision 1"]);
    expect(urls()[0]).toBe(VIEW);
    expect(revisionButtons()[0]).toHaveAttribute("aria-pressed", "true");
    await waitFor(() => expect(paths(filesOf(3))).toEqual([PLAN, "docs/decisions/0053-x.md", "docs/plans/122-x/a.md"]));
    expect(within(filesOf(3)).getByText(/new line/)).toBeInTheDocument();
  });

  it("shows another revision's files when it is chosen", async () => {
    const { revisionReads } = serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    await waitFor(() => expect(revisionButtons()).toHaveLength(3));
    fireEvent.click(screen.getByRole("button", { name: "Revision 1" }));
    await waitFor(() => expect(paths(filesOf(1))).toEqual([PLAN, "docs/plans/122-x/design.md"]));
    expect(screen.getByRole("button", { name: "Revision 1" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Revision 3" })).toHaveAttribute("aria-pressed", "false");
    expect(revisionReads(1)).toBe(1);
  });

  it("a draft that cannot be read says why, and the summary stays", async () => {
    serve(undefined, { view: json({ reason: "unknown_draft" }, 404) });
    render(<DraftSection issue={drafted()} me={colleague} />);
    expect(await screen.findByText("This draft could not be read: Rubato does not know this draft (unknown_draft)")).toBeInTheDocument();
    expect(screen.getByTestId("draft")).toHaveTextContent("owned by alice");
  });
});

describe("what changed since you last looked (per device)", () => {
  it("nothing seen before: the newest revision with no diff, then it counts as seen", async () => {
    serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    expect(await screen.findByText("You have not looked at this draft before; this is revision 3.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "What changed since you last looked" })).toBeNull();
    await waitFor(() => expect(paths(filesOf(3))).toHaveLength(3));
    // With no diff to read, the plan opens.
    expect(filesOf(3).querySelector(".draft-file")).toHaveAttribute("open");
    expect(readSeen(DRAFT)).toBe(3);
  });

  it("seen at revision 1: each file's lines added and removed, files added and removed, then 3 counts as seen", async () => {
    writeSeen(DRAFT, 1);
    const { revisionReads } = serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    const changed = await screen.findByRole("region", { name: "What changed since you last looked" });
    expect(changed).toHaveTextContent("revision 1 → 3");
    await waitFor(() => expect(within(changed).getAllByTestId("draft-change")).toHaveLength(4));
    const rows = within(changed)
      .getAllByTestId("draft-change")
      .map((c) => [c.querySelector("summary code")?.textContent, c.querySelector(".change-status")?.textContent]);
    expect(rows).toEqual([
      [PLAN, "changed · 1 added, 1 removed"],
      ["docs/decisions/0053-x.md", "added · 1 line"],
      ["docs/plans/122-x/a.md", "added · 1 line"],
      ["docs/plans/122-x/design.md", "removed · 1 line"],
    ]);
    const plan = within(changed).getAllByTestId("draft-change")[0] as HTMLElement;
    const added = within(plan).getByText("new line");
    expect(added.tagName).toBe("INS");
    expect(added.closest("li")).toHaveTextContent("added");
    const removed = within(plan).getByText("old line");
    expect(removed.tagName).toBe("DEL");
    expect(removed.closest("li")).toHaveTextContent("removed");
    // The files below start closed when there is a diff to read; the newest revision is read once for both.
    expect(filesOf(3).querySelector(".draft-file")).not.toHaveAttribute("open");
    expect(revisionReads(3)).toBe(1);
    expect(revisionReads(1)).toBe(1);
    expect(readSeen(DRAFT)).toBe(3);
  });

  it("seen at the newest revision: nothing changed", async () => {
    writeSeen(DRAFT, 3);
    serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    expect(await screen.findByText("Nothing changed since you last looked (revision 3).")).toBeInTheDocument();
  });

  it("a seen revision the draft does not have counts as nothing seen", async () => {
    writeSeen(DRAFT, 9);
    serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    expect(await screen.findByText("You have not looked at this draft before; this is revision 3.")).toBeInTheDocument();
  });

  it("works without storage: the newest revision, no diff, no error", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("denied");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    serve();
    render(<DraftSection issue={drafted()} me={colleague} />);
    await waitFor(() => expect(paths(filesOf(3))).toHaveLength(3));
    expect(screen.queryByRole("region", { name: "What changed since you last looked" })).toBeNull();
  });
});

describe("kept current over /ws/web", () => {
  it("an updated event for this draft reads it again and the diff follows the new newest; one for another draft, or for a launch, does not", async () => {
    writeSeen(DRAFT, 1);
    let revs = [1, 2];
    const { viewReads } = serve(() => revs);
    render(<DraftSection issue={drafted({ rev: 2 })} me={colleague} />);
    const changed = await screen.findByRole("region", { name: "What changed since you last looked" });
    expect(changed).toHaveTextContent("revision 1 → 2");
    expect(viewReads()).toBe(1);

    act(() => {
      emitWebEvent({ type: "updated", at: now, resource: { kind: "draft", draft_id: OTHER } });
      emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_log", launch_id: DRAFT } });
    });
    expect(viewReads()).toBe(1);

    revs = [1, 2, 3];
    act(() => emitWebEvent({ type: "updated", at: now, resource: { kind: "draft", draft_id: DRAFT } }));
    expect(viewReads()).toBe(2);
    await waitFor(() => expect(revisionButtons()).toHaveLength(3));
    // The baseline stays what you had seen when you opened it.
    await waitFor(() => expect(screen.getByRole("region", { name: "What changed since you last looked" })).toHaveTextContent("revision 1 → 3"));
    expect(readSeen(DRAFT)).toBe(3);
  });
});

describe("resuming a parked draft (ADR 0053, plan 122 §D5)", () => {
  const owner: Me = { login: "Alice" };

  it("offers its owner a resume on the draft's branch when it is parked", async () => {
    serve();
    const resume = vi.fn();
    render(<DraftSection issue={drafted({ live: false })} me={owner} onResume={resume} />);
    fireEvent.click(await screen.findByRole("button", { name: `Resume on ${BRANCH}` }));
    expect(resume).toHaveBeenCalledWith(BRANCH);
    expect(screen.getByTestId("draft")).toHaveTextContent("revision 3 is written into a new worktree");
  });

  it("offers nothing while it is being worked on, to a teammate, or with no companion of yours", async () => {
    serve();
    const { unmount } = render(<DraftSection issue={drafted({ live: true })} me={owner} onResume={vi.fn()} />);
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
    expect(screen.queryByTestId("resume-draft")).not.toBeInTheDocument();
    unmount();
    const again = render(<DraftSection issue={drafted({ live: false })} me={colleague} onResume={vi.fn()} />);
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
    expect(screen.queryByTestId("resume-draft")).not.toBeInTheDocument();
    again.unmount();
    render(<DraftSection issue={drafted({ live: false })} me={owner} />);
    await screen.findAllByRole("button", { name: /^Revision \d+/ });
    expect(screen.queryByTestId("resume-draft")).not.toBeInTheDocument();
  });
});
