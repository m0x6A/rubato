import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { companion, issue, overview } from "../../../test/fixtures.ts";
import { IssuePanel, ownConnectedCompanions } from "./IssuePanel.tsx";

const me = { login: "m0x6A" };

function renderPanel(ov: ReturnType<typeof overview>, selected = ov.issues[0] ?? null, onLaunch = vi.fn()) {
  render(<IssuePanel overview={ov} me={me} selected={selected} selection={null} onSelect={() => {}} onClearSelection={() => {}} onLaunch={onLaunch} onPin={() => {}} onUnpin={() => {}} />);
  return onLaunch;
}

describe("start session button (ADR 0004: own tmux only)", () => {
  it("is absent when the only connected companion belongs to another user", () => {
    const ov = overview({ issues: [issue(1, "rubato-protocol")], companions: [companion("alice", true)] });
    renderPanel(ov);
    expect(screen.queryByTestId("start-session")).toBeNull();
    expect(screen.getByTestId("no-companion")).toBeInTheDocument();
    expect(ownConnectedCompanions(ov, me)).toEqual([]);
  });

  it("is absent when my companion is paired but not connected, or revoked", () => {
    renderPanel(overview({ issues: [issue(1, "rubato-protocol")], companions: [companion("m0x6A", false), companion("m0x6A", true, { revoked_at: new Date().toISOString() })] }));
    expect(screen.queryByTestId("start-session")).toBeNull();
  });

  it("is present for my own connected companion and launches the selected issue", () => {
    const ov = overview({ issues: [issue(4, "rubato-companion")], companions: [companion("alice", true), companion("m0x6A", true)] });
    const onLaunch = renderPanel(ov);
    const button = screen.getByTestId("start-session");
    expect(button).toBeEnabled();
    fireEvent.click(button);
    expect(onLaunch).toHaveBeenCalledWith({ repo: "m0x6A/rubato", number: 4 });
  });

  it("is disabled for an issue that is not pinned", () => {
    const ov = overview({ issues: [issue(5, "web", { pins: [] })], companions: [companion("m0x6A", true)] });
    renderPanel(ov);
    expect(screen.getByTestId("start-session")).toBeDisabled();
  });
});

describe("issue list", () => {
  it("shows progress badges and sorts blocked first", () => {
    const now = new Date().toISOString();
    const ov = overview({
      issues: [
        issue(1, "web", { progress: { issue: { repo: "m0x6A/rubato", number: 1 }, state: "done", last_seen: now } }),
        issue(2, "web", { progress: { issue: { repo: "m0x6A/rubato", number: 2 }, state: "blocked", last_seen: now, reason: "cap hit" } }),
        issue(3, "web", { progress: { issue: { repo: "m0x6A/rubato", number: 3 }, state: "building", last_seen: now } }),
      ],
    });
    renderPanel(ov, null);
    const rows = screen.getAllByTestId(/^issue-\d+$/).map((el) => el.getAttribute("data-testid"));
    expect(rows).toEqual(["issue-2", "issue-3", "issue-1"]);
    expect(screen.getByText("Blocked")).toBeInTheDocument();
  });
});

describe("spend over time in the instruments (issue #103)", () => {
  it("draws the sparkline after the gauge for an issue with history, and none without", () => {
    const history = [
      { at: "2026-11-13T08:00:00Z", usd: 1 },
      { at: "2026-11-13T08:15:00Z", usd: 4 },
    ];
    const withCap = issue(1, "web", { spend_usd: 4, spend_history: history, plan: { issue: { repo: "m0x6A/rubato", number: 1 }, path: "docs/plans/1.md", affected_parts: [], spend_cap_usd: 25 } });
    const { container, unmount } = render(<IssuePanel overview={overview({ issues: [withCap] })} me={me} selected={withCap} selection={null} onSelect={() => {}} onClearSelection={() => {}} onLaunch={() => {}} onPin={() => {}} onUnpin={() => {}} />);
    const kids = [...container.querySelector(".instruments")!.children].map((el) => el.className);
    expect(kids.indexOf("sparkline")).toBeGreaterThan(kids.indexOf("gauge"));
    expect(kids.indexOf("gauge")).toBeGreaterThanOrEqual(0);
    unmount();
    const without = issue(2, "web");
    renderPanel(overview({ issues: [without] }), without);
    expect(document.querySelector(".instruments .sparkline")).toBeNull();
  });
});

describe("pins on the issue detail and the part filter (N18)", () => {
  it("unpin posts a delete", () => {
    const onUnpin = vi.fn();
    const ov = overview({ issues: [issue(7, "web", { pins: [{ kind: "node", id: "web" }, { kind: "seam", id: "web-server" }] })] });
    render(<IssuePanel overview={ov} me={me} selected={ov.issues[0] ?? null} selection={null} onSelect={() => {}} onClearSelection={() => {}} onLaunch={() => {}} onPin={() => {}} onUnpin={onUnpin} />);
    const buttons = screen.getAllByRole("button", { name: "unpin" });
    expect(buttons).toHaveLength(2);
    fireEvent.click(buttons[1] as HTMLElement);
    expect(onUnpin).toHaveBeenCalledTimes(1);
    expect(onUnpin).toHaveBeenCalledWith({ issue: { repo: "m0x6A/rubato", number: 7 }, target: { kind: "seam", id: "web-server" } });
  });

  it("a part filter lists its issues", () => {
    const onClear = vi.fn();
    const ov = overview({ issues: [issue(1, "web"), issue(2, "rubato-server"), issue(3, "web")] });
    render(<IssuePanel overview={ov} me={me} selected={null} selection={{ kind: "node", id: "web" }} onSelect={() => {}} onClearSelection={onClear} onLaunch={() => {}} onPin={() => {}} onUnpin={() => {}} />);
    expect(screen.getAllByTestId(/^issue-\d+$/).map((el) => el.getAttribute("data-testid"))).toEqual(["issue-1", "issue-3"]);
    fireEvent.click(screen.getByRole("button", { name: "show all" }));
    expect(onClear).toHaveBeenCalledTimes(1);
  });
});

describe("seam selection (S4)", () => {
  // A part and a seam may share an id; the filter must not confuse them.
  const ov = () =>
    overview({
      issues: [issue(1, "web"), issue(2, "web", { pins: [{ kind: "seam", id: "web" }] }), issue(3, "web", { pins: [{ kind: "seam", id: "web-server" }] }), issue(4, "web", { pins: [{ kind: "node", id: "web" }, { kind: "seam", id: "web-server" }] })],
    });
  const listed = () => screen.queryAllByTestId(/^issue-\d+$/).map((el) => el.getAttribute("data-testid"));
  const panel = (selection: { kind: "node" | "seam"; id: string }) =>
    render(<IssuePanel overview={ov()} me={me} selected={null} selection={selection} onSelect={() => {}} onClearSelection={() => {}} onLaunch={() => {}} onPin={() => {}} onUnpin={() => {}} />);

  it("a seam filter matches only seam pins", () => {
    const { unmount } = panel({ kind: "seam", id: "web-server" });
    expect(listed()).toEqual(["issue-3", "issue-4"]);
    expect(screen.getByTestId("map-filter")).toHaveTextContent("pinned to seam web-server");
    unmount();

    panel({ kind: "seam", id: "web" });
    expect(listed()).toEqual(["issue-2"]);
  });

  it("a part filter no longer matches a seam pin with the same id", () => {
    panel({ kind: "node", id: "web" });
    expect(listed()).toEqual(["issue-1", "issue-4"]);
  });

  it("says which part or seam has no issue instead of an empty list", () => {
    panel({ kind: "seam", id: "companion-server" });
    expect(listed()).toEqual([]);
    expect(screen.getByTestId("no-tracked-issues")).toHaveTextContent("No issue is pinned to seam companion-server.");
  });
});

describe("pinning from the list (S3)", () => {
  it("a tracked row is draggable and its Pin to… menu adds a pin", async () => {
    const { dataTransfer } = await import("../../../test/dnd.ts");
    const { getIssueDrag } = await import("../../shared/dnd.ts");
    const onPin = vi.fn();
    const onSelect = vi.fn();
    const ov = overview({ issues: [issue(7, "web")] });
    render(<IssuePanel overview={ov} me={me} selected={null} selection={null} onSelect={onSelect} onClearSelection={() => {}} onLaunch={() => {}} onPin={onPin} onUnpin={() => {}} />);
    const row = screen.getByTestId("issue-7");
    const dt = dataTransfer();
    fireEvent.dragStart(row, { dataTransfer: dt });
    expect(getIssueDrag({ dataTransfer: dt })).toEqual({ repo: "m0x6A/rubato", number: 7 });

    fireEvent.click(screen.getByRole("button", { name: "Pin m0x6A/rubato#7 to…" }));
    // Where it is pinned already is shown and cannot be chosen again.
    const here = screen.getByRole("menuitem", { name: /^web\b.*pinned here/ });
    expect(here).toHaveAttribute("aria-disabled", "true");
    fireEvent.click(here);
    expect(onPin).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("menuitem", { name: /^rubato-server\b/ }));
    expect(onPin).toHaveBeenCalledWith({ repo: "m0x6A/rubato", number: 7 }, { kind: "node", id: "rubato-server" });
    // Using the menu does not open the issue.
    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("opening an issue", () => {
  it("moves focus to the issue's heading", () => {
    const ov = overview({ issues: [issue(7, "web")] });
    const props = { overview: ov, me, selection: null, onSelect: () => {}, onClearSelection: () => {}, onLaunch: () => {}, onPin: () => {}, onUnpin: () => {} };
    const { rerender } = render(<IssuePanel {...props} selected={null} />);
    expect(document.body).toHaveFocus();
    rerender(<IssuePanel {...props} selected={ov.issues[0] ?? null} />);
    const heading = screen.getByRole("heading", { name: "m0x6A/rubato#7" });
    expect(heading).toHaveAttribute("tabindex", "-1");
    expect(heading).toHaveFocus();
  });
});
