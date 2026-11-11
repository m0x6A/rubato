import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { dataTransfer } from "../../../test/dnd.ts";
import { companion, issue, minutesAgo, openIssue, overview } from "../../../test/fixtures.ts";
import type { IssueRef, Overview, QueuedIssue, SessionState } from "../../protocol.ts";
import { setIssueDrag } from "../../shared/dnd.ts";
import { QueueMenu } from "../queue/index.ts";
import { Lanes, orderLanes, shownStatus } from "./Lanes.tsx";

const me = { login: "m0x6A" };
const REPO = "m0x6A/rubato";
const MINE = "11111111-1111-1111-1111-111111111111";
const MINE_OFF = "33333333-3333-3333-3333-333333333333";
const ALICE = "22222222-2222-2222-2222-222222222222";
const mine = companion("m0x6A", true, { companion_id: MINE, hostname: "my-laptop" });
const mineOff = companion("m0x6A", false, { companion_id: MINE_OFF, hostname: "my-desktop", last_seen: minutesAgo(90) });
const alices = companion("alice", true, { companion_id: ALICE, hostname: "alice-box" });

const queued = (number: number, companion_id: string, position: number, extra: Partial<QueuedIssue> = {}): QueuedIssue => ({ issue: { repo: REPO, number }, companion_id, queued_by: "m0x6A", queued_at: minutesAgo(5), position, ...extra });
const session = (companion_id: string, extra: Partial<SessionState> = {}): SessionState => ({ companion_id, cwd: "/w", last_seen: new Date().toISOString(), source: "hook", status: "working", user: "m0x6A", ...extra });

function show(ov: Overview) {
  const actions = { onQueue: vi.fn(), onReorder: vi.fn(), onUnqueue: vi.fn(), onStart: vi.fn(), onRevoke: vi.fn() };
  render(<Lanes overview={ov} me={me} {...actions} />);
  return actions;
}
const lane = (id: string) => screen.getByTestId(`lane-${id}`);
function dragOf(ref: IssueRef): DataTransfer {
  const dt = dataTransfer();
  setIssueDrag({ dataTransfer: dt }, ref);
  return dt;
}

describe("companion lanes (S13)", () => {
  it("own lanes first", () => {
    const revoked = companion("m0x6A", false, { companion_id: "44444444-4444-4444-4444-444444444444", hostname: "old", revoked_at: minutesAgo(10) });
    expect(orderLanes([alices, revoked, mineOff, mine], me).map((c) => c.hostname)).toEqual(["my-desktop", "my-laptop", "alice-box", "old"]);
    show(overview({ companions: [alices, mine] }));
    const lanes = within(screen.getByTestId("lanes")).getAllByRole("listitem").filter((li) => li.dataset["testid"]?.startsWith("lane-"));
    expect(lanes.map((l) => l.getAttribute("aria-label"))).toEqual(["Your companion my-laptop", "alice's companion alice-box"]);
    expect(lane(MINE)).toHaveTextContent("my-laptop yours");
    expect(lane(MINE)).toHaveTextContent("connected");
    expect(lane(ALICE)).toHaveTextContent("alice-box alice");
    // Revoke is offered only on your own.
    expect(within(lane(MINE)).getByRole("button", { name: "revoke" })).toBeInTheDocument();
    expect(within(lane(ALICE)).queryByRole("button", { name: "revoke" })).toBeNull();
  });

  it("a stale session shows unknown", () => {
    const now = Date.now();
    expect(shownStatus(session(MINE, { last_seen: new Date(now - 10_000).toISOString() }), now)).toBe("working");
    expect(shownStatus(session(MINE, { last_seen: new Date(now - 121_000).toISOString() }), now)).toBe("unknown");
    // Only "working" needs a flowing signal: an idle or ended session stays what it reported.
    expect(shownStatus(session(MINE, { status: "idle", last_seen: new Date(now - 900_000).toISOString() }), now)).toBe("idle");

    const ref = { repo: REPO, number: 48 };
    show(overview({ companions: [mine], issues: [issue(48, "web", { sessions: [session(MINE, { issue: ref, last_seen: minutesAgo(30), spend_usd: 1.5 })] })] }));
    const row = within(lane(MINE)).getByTestId("lane-session");
    expect(row).toHaveTextContent("m0x6A/rubato#48");
    expect(row).toHaveTextContent("unknown");
    expect(row).not.toHaveTextContent("working");
    expect(row).toHaveTextContent("$1.50");
  });

  it("sessions without an issue are listed", () => {
    show(overview({ companions: [mine, alices], unpinned_sessions: [session(MINE, { branch: "scratch", status: "idle" })] }));
    const row = within(lane(MINE)).getByTestId("lane-session");
    expect(row).toHaveTextContent("no issue (branch scratch)");
    expect(row).toHaveTextContent("idle");
    expect(lane(ALICE)).toHaveTextContent("No session reported.");
  });
});

describe("the queue on a lane (S14 to S16)", () => {
  it("drop on own lane queues", () => {
    const actions = show(overview({ companions: [mine, alices] }));
    const dt = dragOf({ repo: REPO, number: 902 });
    fireEvent.dragOver(lane(MINE), { dataTransfer: dt });
    expect(lane(MINE)).toHaveClass("drop-over");
    expect(lane(MINE)).toHaveTextContent("Drop to queue here");
    fireEvent.drop(lane(MINE), { dataTransfer: dt });
    expect(actions.onQueue).toHaveBeenCalledWith({ repo: REPO, number: 902 }, MINE);
    expect(lane(MINE)).not.toHaveClass("drop-over");
  });

  it("Queue on… queues by keyboard", () => {
    const onQueue = vi.fn();
    const ref = { repo: REPO, number: 902 };
    render(<QueueMenu issue={ref} lanes={[mine, mineOff]} queue={[queued(902, MINE_OFF, 0)]} onQueue={onQueue} />);
    const trigger = screen.getByRole("button", { name: "Queue m0x6A/rubato#902 on…" });
    fireEvent.click(trigger);
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual(["my-laptopconnected", "my-desktopqueued here"]);
    expect(items[0]).toHaveFocus();
    // Where it is queued already cannot be chosen again.
    fireEvent.keyDown(items[0]!, { key: "ArrowDown" });
    expect(items[1]).toHaveFocus();
    fireEvent.click(items[1]!);
    expect(onQueue).not.toHaveBeenCalled();
    fireEvent.keyDown(items[1]!, { key: "Home" });
    fireEvent.click(document.activeElement as HTMLElement);
    expect(onQueue).toHaveBeenCalledWith(ref, MINE);
    expect(screen.queryByRole("menu")).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it("someone with no companion is offered no Queue on…", () => {
    const { container } = render(<QueueMenu issue={{ repo: REPO, number: 1 }} lanes={[]} queue={[]} onQueue={() => {}} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("another user's lane rejects the drop", () => {
    const actions = show(overview({ companions: [mine, alices], queue: [queued(7, ALICE, 0, { queued_by: "alice" })] }));
    const dt = dragOf({ repo: REPO, number: 902 });
    const over = fireEvent.dragOver(lane(ALICE), { dataTransfer: dt });
    // Not prevented: the browser delivers no drop there.
    expect(over).toBe(true);
    expect(lane(ALICE)).toHaveClass("drop-refused");
    expect(within(lane(ALICE)).getByRole("status")).toHaveTextContent("This is alice's companion: you can queue issues only on your own.");
    fireEvent.drop(lane(ALICE), { dataTransfer: dt });
    expect(actions.onQueue).not.toHaveBeenCalled();
    // Everyone sees the queue; only its owner gets the controls.
    const entry = within(lane(ALICE)).getByTestId("queued-7");
    expect(entry).toHaveTextContent("queued by alice");
    expect(within(entry).queryByRole("button")).toBeNull();
    expect(entry).toHaveAttribute("draggable", "false");
  });

  it("Start is disabled with 'pin first' for an unpinned issue", () => {
    const actions = show(
      overview({
        companions: [mine],
        issues: [issue(48, "web", { title: "Interactive map" })],
        backlog: { issues: [openIssue(902, "Backlog: an unpinned issue")], repos: [] },
        queue: [queued(48, MINE, 0), queued(902, MINE, 1)],
      }),
    );
    const pinned = within(lane(MINE)).getByTestId("queued-48");
    const unpinned = within(lane(MINE)).getByTestId("queued-902");
    expect(pinned).toHaveTextContent("m0x6A/rubato#48 Interactive map");
    expect(unpinned).toHaveTextContent("m0x6A/rubato#902 Backlog: an unpinned issue");
    expect(unpinned).toHaveTextContent("pin first");
    const blocked = within(unpinned).getByRole("button", { name: "Start m0x6A/rubato#902" });
    expect(blocked).toBeDisabled();
    expect(blocked).toHaveAttribute("title", "pin first");
    // The pinned one starts, on this lane's companion, with nothing but the issue.
    fireEvent.click(within(pinned).getByRole("button", { name: "Start m0x6A/rubato#48" }));
    expect(actions.onStart).toHaveBeenCalledWith({ repo: REPO, number: 48 }, MINE);
    // Reorder and remove from the keyboard.
    expect(within(pinned).getByRole("button", { name: "Move m0x6A/rubato#48 up" })).toBeDisabled();
    fireEvent.click(within(pinned).getByRole("button", { name: "Move m0x6A/rubato#48 down" }));
    expect(actions.onReorder).toHaveBeenCalledWith(MINE, [
      { repo: REPO, number: 902 },
      { repo: REPO, number: 48 },
    ]);
    fireEvent.click(within(unpinned).getByRole("button", { name: "Remove m0x6A/rubato#902 from the queue" }));
    expect(actions.onUnqueue).toHaveBeenCalledWith({ repo: REPO, number: 902 }, MINE);
  });

  it("Start is disabled for a disconnected lane", () => {
    show(overview({ companions: [mineOff], issues: [issue(48, "web")], queue: [queued(48, MINE_OFF, 0)] }));
    expect(lane(MINE_OFF)).toHaveTextContent("last seen 2h ago");
    const start = within(lane(MINE_OFF)).getByRole("button", { name: "Start m0x6A/rubato#48" });
    expect(start).toBeDisabled();
    expect(start).toHaveAttribute("title", "companion not connected");
    expect(within(lane(MINE_OFF)).getByTestId("queued-48")).toHaveTextContent("companion not connected");
  });

  it("a refused launch keeps the entry and shows the reason", () => {
    show(overview({ companions: [mine], issues: [issue(48, "web")], queue: [queued(48, MINE, 0, { last_start_error: "missing_dependency: tmux-ide 2.9 is needed" })] }));
    const entry = within(lane(MINE)).getByTestId("queued-48");
    expect(entry).toHaveTextContent("did not start: missing_dependency: tmux-ide 2.9 is needed");
    // Still queued, and it can be started again.
    expect(within(entry).getByRole("button", { name: "Start m0x6A/rubato#48" })).toBeEnabled();
  });

  it("dragging an entry onto another reorders the lane", () => {
    const actions = show(overview({ companions: [mine], issues: [issue(1, "web"), issue(2, "web"), issue(3, "web")], queue: [queued(1, MINE, 0), queued(2, MINE, 1), queued(3, MINE, 2)] }));
    const dt = dataTransfer();
    fireEvent.dragStart(within(lane(MINE)).getByTestId("queued-3"), { dataTransfer: dt });
    fireEvent.drop(within(lane(MINE)).getByTestId("queued-1"), { dataTransfer: dt });
    expect(actions.onReorder).toHaveBeenCalledWith(MINE, [3, 1, 2].map((number) => ({ repo: REPO, number })));
    // A reorder is not a new queue entry.
    expect(actions.onQueue).not.toHaveBeenCalled();
  });
});
