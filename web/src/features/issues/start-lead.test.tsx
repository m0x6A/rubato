import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { companion, issue, overview } from "../../../test/fixtures.ts";
import { IssuePanel } from "../../../test/IssuePanel.tsx";

const me = { login: "m0x6A" };
const MINE = "11111111-1111-1111-1111-111111111111";

describe("Start as lead agent (ADR 0072)", () => {
  const pinned = issue(4, "web");
  function panel(orchestrator: boolean, can_lead: boolean, onLaunchLead = vi.fn()) {
    const ov = overview({ issues: [pinned], companions: [companion("m0x6A", true, { companion_id: MINE, hostname: "box", can_lead })], orchestrator });
    render(<IssuePanel overview={ov} me={me} selected={pinned} selection={null} onSelect={() => {}} onClearSelection={() => {}} onLaunch={() => {}} onLaunchLead={onLaunchLead} onPin={() => {}} onUnpin={() => {}} />);
    return onLaunchLead;
  }

  it("is offered only with the server's flag on and a companion of yours that runs leads", () => {
    const go = panel(true, true);
    fireEvent.click(screen.getByTestId("start-lead"));
    expect(go).toHaveBeenCalledWith({ repo: "m0x6A/rubato", number: 4 }, MINE);
  });

  it("is not offered with the flag off", () => {
    panel(false, true);
    expect(screen.queryByTestId("start-lead")).toBeNull();
    expect(screen.getByTestId("start-session")).toBeInTheDocument();
  });

  it("is not offered when no companion of yours runs leads", () => {
    panel(true, false);
    expect(screen.queryByTestId("start-lead")).toBeNull();
  });
});
