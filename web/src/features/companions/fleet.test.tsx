import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { companion, issue, overview } from "../../../test/fixtures.ts";
import type { SessionState } from "../../protocol.ts";
import { IssuePanel } from "../issues/index.ts";
import { Lanes } from "./Lanes.tsx";
import { AUTH_LABEL, fleetParts } from "./Lanes.tsx";

const me = { login: "m0x6A" };
const MINE = "11111111-1111-1111-1111-111111111111";
const LEAD = "aaaaaaaa-0000-4000-8000-000000000001";
const WORKER = "aaaaaaaa-0000-4000-8000-000000000002";
const OTHER = "aaaaaaaa-0000-4000-8000-000000000003";
const session = (launch_id: string, extra: Partial<SessionState> = {}): SessionState => ({ companion_id: MINE, cwd: "/w", last_seen: new Date().toISOString(), source: "hook", status: "working", user: "m0x6A", launch_id, ...extra });

function lanes(ov: ReturnType<typeof overview>) {
  render(<Lanes overview={ov} me={me} onQueue={vi.fn()} onReorder={vi.fn()} onUnqueue={vi.fn()} onStart={vi.fn()} onRevoke={vi.fn()} />);
}

describe("a person's own fleet on the lanes (ADR 0072)", () => {
  it("marks the lead and the workers it launched, and nothing while there are no leads", () => {
    const ov = overview({
      companions: [companion("m0x6A", true, { companion_id: MINE, hostname: "box" })],
      unpinned_sessions: [session(LEAD), session(WORKER), session(OTHER)],
      leads: [{ launch_id: LEAD, owner: "m0x6A", workers: [WORKER] }],
    });
    lanes(ov);
    expect(screen.getAllByTestId("fleet-lead")).toHaveLength(1);
    expect(screen.getAllByTestId("fleet-worker")).toHaveLength(1);
    expect(screen.getByTestId("fleet-worker")).toHaveTextContent("via lead");
    expect(fleetParts(ov).get(OTHER)).toBeUndefined();
  });

  it("shows no fleet chip when the overview carries no leads (the flag off)", () => {
    lanes(overview({ companions: [companion("m0x6A", true, { companion_id: MINE, hostname: "box" })], unpinned_sessions: [session(LEAD)] }));
    expect(screen.queryByTestId("fleet-lead")).toBeNull();
    expect(screen.queryByTestId("fleet-worker")).toBeNull();
  });
});

describe("how a companion's Claude Code is billed (ADR 0076)", () => {
  it("names an API key, Bedrock or Vertex, and nothing for the developer's own login", () => {
    lanes(
      overview({
        companions: [
          companion("m0x6A", true, { companion_id: MINE, hostname: "box", claude_auth: "api_key" }),
          companion("alice", true, { companion_id: "22222222-2222-2222-2222-222222222222", hostname: "alice", claude_auth: "login" }),
        ],
      }),
    );
    expect(screen.getByTestId(`auth-${MINE}`)).toHaveTextContent("API key");
    expect(screen.queryByTestId("auth-22222222-2222-2222-2222-222222222222")).toBeNull();
    expect(AUTH_LABEL["bedrock"]).toBe("Bedrock");
  });
});

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
