import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { IssuePanel, ownConnectedCompanions } from "../src/components/IssuePanel.tsx";
import { companion, issue, overview } from "./fixtures.ts";

const me = { login: "m0x6A" };

function renderPanel(ov: ReturnType<typeof overview>, selected = ov.issues[0] ?? null, onLaunch = vi.fn()) {
  render(<IssuePanel overview={ov} me={me} selected={selected} selectedNode={null} onSelect={() => {}} onClearNode={() => {}} onLaunch={onLaunch} onUnpin={() => {}} />);
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
