/**
 * Calling an agent into a plan's room (ADR 0057, PR 11): presence, the owner's call-in and a
 * teammate's request, answering and withdrawing it, dismissing, addressing an agent, and an
 * agent's reply under the turn it answers.
 */
import { configure, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue } from "../../../test/fixtures.ts";
import type { IssueSummary, Me, RoomAgent, RoomMessage, RoomRequest, RoomView, SessionState } from "../../protocol.ts";
import { ROOM_MAX_CHARS, RoomSection } from "./index.ts";

afterEach(() => vi.restoreAllMocks());
configure({ asyncUtilTimeout: 5000 });

const REPO = "m0x6A/rubato";
const BRANCH = "issue/122-plan-in-rubato";
const LAUNCH = "6f1c0c1e-0000-4000-8000-0000000000a1";
const now = new Date().toISOString();
const minutesAgo = (n: number) => new Date(Date.now() - n * 60_000).toISOString();
const alice: Me = { login: "alice" };
const bob: Me = { login: "bob" };
const carol: Me = { login: "carol" };

const session = (extra: Partial<SessionState> = {}): SessionState => ({ companion_id: "c1", user: "alice", cwd: "/w/122", branch: BRANCH, launch_id: LAUNCH, source: "hook", status: "idle", last_seen: now, spend_usd: 4.2, ...extra });
const draft = { draft_id: "d0000000-0000-4000-8000-000000000001", branch: BRANCH, owner: "alice", rev: 2, at: now, plan_sha: "p", bundle_hash: "h", live: true };
/** Planning: a draft, no plan on the branch. */
const planning = (sessions: SessionState[] = [session()], extra: Partial<IssueSummary> = {}): IssueSummary => issue(122, "web", { draft, sessions, ...extra });
/** Building: the plan is on the branch. */
const building = (sessions: SessionState[] = [session()]): IssueSummary => issue(122, "web", { sessions, plan: { issue: { repo: REPO, number: 122 }, path: "docs/plans/122.md", affected_parts: [], git_ref: BRANCH } });

const agent = (extra: Partial<RoomAgent> = {}): RoomAgent => ({ handle: "alice-agent", owner: "alice", launch_id: LAUNCH, called_by: "alice", joined_at: minutesAgo(5), ...extra });
const request = (extra: Partial<RoomRequest> = {}): RoomRequest => ({ id: "r1", launch_id: LAUNCH, owner: "alice", requested_by: "bob", at: minutesAgo(2), state: "pending", ...extra });
const message = (id: string, extra: Partial<RoomMessage> = {}): RoomMessage => ({ id, author: "bob", author_kind: "person", text: `text ${id}`, at: now, ...extra });
const view = (extra: Partial<RoomView> = {}): RoomView => ({ repo: REPO, number: 122, branch: BRANCH, messages: [], agents: [], requests: [], ...extra });
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

type Answer = (body: Record<string, unknown>) => Response;

/** A server holding the room; call-in, dismiss and answers reply with what the test gives. */
function serve(room: () => RoomView, on: { callIn?: Answer; dismiss?: Answer; answer?: Answer } = {}) {
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const body = init?.body ? (JSON.parse(init.body as string) as Record<string, unknown>) : {};
    if (method === "GET" && url.startsWith("/api/room?")) {
      return json(room());
    }
    if (url === "/api/room/agents" && method === "POST") {
      return (on.callIn ?? (() => json({ outcome: "joined", agent: agent() })))(body);
    }
    if (url === "/api/room/agents" && method === "DELETE") {
      return (on.dismiss ?? (() => new Response(null, { status: 204 })))(body);
    }
    if (url.startsWith("/api/room/requests/") && method === "POST") {
      return (on.answer ?? (() => new Response(null, { status: 204 })))(body);
    }
    return json({ reason: "unexpected" }, 500);
  });
  const calls = (method: string, url: string) =>
    fetch.mock.calls.filter((c) => (c[1]?.method ?? "GET") === method && String(c[0]) === url).map((c) => JSON.parse((c[1]?.body as string) ?? "{}") as Record<string, unknown>);
  return { calls, roomReads: () => fetch.mock.calls.filter((c) => String(c[0]).startsWith("/api/room?")).length };
}

const presence = () => screen.getByRole("region", { name: "Agents" });

describe("who is in the room, called in by whom, since when; who left and why", () => {
  it("lists the agents in the room and those who left, with the reason", async () => {
    const joined = minutesAgo(5);
    serve(() =>
      view({
        agents: [
          agent({ called_by: "bob", joined_at: joined }),
          agent({ handle: "carol-agent", owner: "carol", launch_id: "l2", called_by: "carol", joined_at: minutesAgo(90), left_at: minutesAgo(40), left_reason: "timed_out" }),
          agent({ handle: "dave-agent", owner: "dave", launch_id: "l3", called_by: "dave", left_at: minutesAgo(3), left_reason: "dismissed" }),
          agent({ handle: "erin-agent", owner: "erin", launch_id: "l4", called_by: "erin", left_at: minutesAgo(1), left_reason: "session_ended" }),
        ],
      }),
    );
    render(<RoomSection issue={planning()} me={carol} maxChars={ROOM_MAX_CHARS} />);
    const inRoom = await within(await screen.findByRole("region", { name: "Agents" })).findByRole("list", { name: "In the room" });
    const here = within(inRoom).getAllByRole("listitem");
    expect(here).toHaveLength(1);
    expect(here[0]).toHaveTextContent("alice-agent");
    expect(here[0]).toHaveTextContent("called in by bob");
    expect(here[0]?.querySelector("time")).toHaveAttribute("datetime", joined);
    const left = within(presence()).getByRole("list", { name: "Left the room" });
    const words = within(left)
      .getAllByRole("listitem")
      .map((li) => li.textContent ?? "");
    expect(words[0]).toMatch(/carol-agent.*not addressed for 30 minutes/);
    expect(words[1]).toMatch(/dave-agent.*dismissed/);
    expect(words[2]).toMatch(/erin-agent.*its session ended/);
  });

  it("says nobody is in when no agent was called", async () => {
    serve(() => view());
    render(<RoomSection issue={planning([])} me={carol} maxChars={ROOM_MAX_CHARS} />);
    expect(await within(await screen.findByRole("region", { name: "Agents" })).findByText("No agent is in the room. An agent joins only when its owner calls it in.")).toBeInTheDocument();
  });
});

describe("calling an agent in: the owner calls, a teammate asks", () => {
  it("the owner of the launched session calls their own agent in, after the estimate; it joins", async () => {
    let state = view();
    const { calls, roomReads } = serve(() => state, {
      callIn: () => {
        state = view({ agents: [agent()] });
        return json({ outcome: "joined", agent: agent() });
      },
    });
    render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    const open = await screen.findByRole("button", { name: "Call my agent in" });
    expect(screen.queryByRole("button", { name: /^Ask / })).toBeNull();
    fireEvent.click(open);
    const dialog = screen.getByRole("group", { name: "Call your agent in?" });
    expect(dialog).toHaveTextContent("typed into your working session");
    const confirm = within(dialog).getByRole("button", { name: "Call in" });
    expect(confirm).toHaveFocus();
    fireEvent.click(confirm);
    await waitFor(() => expect(calls("POST", "/api/room/agents")).toEqual([{ repo: REPO, number: 122, branch: BRANCH, launch_id: LAUNCH }]));
    expect(await screen.findByRole("status")).toHaveTextContent("alice-agent joined the room.");
    await waitFor(() => expect(roomReads()).toBeGreaterThanOrEqual(2));
    // In the room now: no second call-in for that session.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Call my agent in" })).toBeNull());
  });

  it("Cancel closes the estimate and puts focus back on the button that opened it", async () => {
    const { calls } = serve(() => view());
    render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    const open = await screen.findByRole("button", { name: "Call my agent in" });
    fireEvent.click(open);
    fireEvent.click(within(screen.getByRole("group", { name: "Call your agent in?" })).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("group", { name: "Call your agent in?" })).toBeNull();
    expect(screen.getByRole("button", { name: "Call my agent in" })).toHaveFocus();
    expect(calls("POST", "/api/room/agents")).toEqual([]);
  });

  it("a teammate asks the owner instead, and the request shows as pending", async () => {
    let state = view();
    const { calls } = serve(() => state, {
      callIn: () => {
        state = view({ requests: [request()] });
        return json({ outcome: "requested", request: request() });
      },
    });
    render(<RoomSection issue={planning()} me={bob} maxChars={ROOM_MAX_CHARS} />);
    expect(screen.queryByRole("button", { name: "Call my agent in" })).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "Ask alice to call their agent in" }));
    const dialog = screen.getByRole("group", { name: "Ask alice to call their agent in?" });
    expect(dialog).toHaveTextContent("alice decides");
    expect(dialog).toHaveTextContent("typed into alice's working session");
    fireEvent.click(within(dialog).getByRole("button", { name: "Ask" }));
    await waitFor(() => expect(calls("POST", "/api/room/agents")).toEqual([{ repo: REPO, number: 122, branch: BRANCH, launch_id: LAUNCH }]));
    expect(await screen.findByRole("status")).toHaveTextContent("Asked alice; the request is pending.");
    const pending = await within(presence()).findByTestId("room-request-r1");
    expect(pending).toHaveTextContent("bob asked alice to call their agent in");
    expect(pending).toHaveTextContent("pending");
    expect(within(pending).getByRole("button", { name: "Withdraw" })).toBeInTheDocument();
    expect(within(pending).queryByRole("button", { name: "Accept" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Ask alice to call their agent in" })).toBeNull();
  });

  it("no call-in without a launched session on this branch, for an agent already in, or for someone not logged in", async () => {
    serve(() => view({ agents: [agent({ launch_id: "other-launch", handle: "dave-agent", owner: "dave" })] }));
    const { unmount } = render(<RoomSection issue={planning([session({ branch: "issue/122-elsewhere" }), session({ launch_id: null, companion_id: "c2" })])} me={alice} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByRole("region", { name: "Agents" });
    expect(screen.queryByRole("button", { name: /Call my agent in|^Ask / })).toBeNull();
    unmount();
    vi.restoreAllMocks();
    serve(() => view({ agents: [agent()] }));
    const second = render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    await within(await screen.findByRole("region", { name: "Agents" })).findByText("alice-agent");
    expect(screen.queryByRole("button", { name: "Call my agent in" })).toBeNull();
    second.unmount();
    render(<RoomSection issue={planning()} me={null} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByRole("region", { name: "Agents" });
    expect(screen.queryByRole("button", { name: /Call my agent in|^Ask / })).toBeNull();
  });
});

describe("the call-in estimate", () => {
  it("with the plan on the branch: a first fork turn costs about the session's spend so far", async () => {
    serve(() => view());
    render(<RoomSection issue={building()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    fireEvent.click(await screen.findByRole("button", { name: "Call my agent in" }));
    const dialog = screen.getByRole("group", { name: "Call your agent in?" });
    expect(dialog).toHaveTextContent("A first turn re-reads the session uncached: about $4.20");
    expect(dialog).toHaveTextContent("your spend cap");
    expect(dialog).not.toHaveTextContent("typed into");
  });

  it("with the plan on the branch and no spend known, it says so", async () => {
    serve(() => view());
    render(<RoomSection issue={building([session({ spend_usd: null })])} me={alice} maxChars={ROOM_MAX_CHARS} />);
    fireEvent.click(await screen.findByRole("button", { name: "Call my agent in" }));
    expect(screen.getByRole("group", { name: "Call your agent in?" })).toHaveTextContent("its spend so far is not known");
  });

  it("while planning: typed into the working session", async () => {
    serve(() => view());
    render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    fireEvent.click(await screen.findByRole("button", { name: "Call my agent in" }));
    expect(screen.getByRole("group", { name: "Call your agent in?" })).toHaveTextContent("Its turns are typed into your working session when it next waits for input");
  });
});

describe("answering a request: the owner accepts or declines, the asker withdraws", () => {
  it("the owner accepts or declines a pending request", async () => {
    const { calls } = serve(() => view({ requests: [request(), request({ id: "r2", requested_by: "carol" })] }));
    render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    const first = await screen.findByTestId("room-request-r1");
    expect(first).toHaveTextContent("bob asks you to call your agent in");
    fireEvent.click(within(first).getByRole("button", { name: "Accept" }));
    await waitFor(() => expect(calls("POST", "/api/room/requests/r1")).toEqual([{ answer: "accept" }]));
    expect(await screen.findByRole("status")).toHaveTextContent("Accepted");
    fireEvent.click(within(screen.getByTestId("room-request-r2")).getByRole("button", { name: "Decline" }));
    await waitFor(() => expect(calls("POST", "/api/room/requests/r2")).toEqual([{ answer: "decline" }]));
    expect(within(screen.getByTestId("room-request-r1")).queryByRole("button", { name: "Withdraw" })).toBeNull();
  });

  it("the asker withdraws their own request; a third person only sees it pending", async () => {
    const { calls } = serve(() => view({ requests: [request()] }));
    const { unmount } = render(<RoomSection issue={planning()} me={bob} maxChars={ROOM_MAX_CHARS} />);
    fireEvent.click(within(await screen.findByTestId("room-request-r1")).getByRole("button", { name: "Withdraw" }));
    await waitFor(() => expect(calls("POST", "/api/room/requests/r1")).toEqual([{ answer: "withdraw" }]));
    unmount();
    render(<RoomSection issue={planning()} me={carol} maxChars={ROOM_MAX_CHARS} />);
    const seen = await screen.findByTestId("room-request-r1");
    expect(seen).toHaveTextContent("pending");
    expect(within(seen).queryAllByRole("button")).toHaveLength(0);
  });

  it("an answered request reads as what happened, with no buttons", async () => {
    serve(() => view({ requests: [request({ state: "declined", answered_at: minutesAgo(1) })] }));
    render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    const r = await screen.findByTestId("room-request-r1");
    expect(r).toHaveTextContent("declined");
    expect(within(r).queryAllByRole("button")).toHaveLength(0);
  });

  it("a refusal says why, in words with its code", async () => {
    serve(() => view({ requests: [request()] }), { answer: () => json({ reason: "answered" }, 409), callIn: () => json({ reason: "companion_cannot" }, 409) });
    render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    fireEvent.click(within(await screen.findByTestId("room-request-r1")).getByRole("button", { name: "Accept" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Not done: That request was answered already (answered)");
    fireEvent.click(screen.getByRole("button", { name: "Call my agent in" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Call your agent in?" })).getByRole("button", { name: "Call in" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Not done: The owner's companion cannot take room turns yet; it needs an upgrade (companion_cannot)"));
  });
});

describe("dismissing an agent: its owner, or whoever called it in", () => {
  it("the owner and the caller may dismiss; anyone else may not", async () => {
    const { calls } = serve(() => view({ agents: [agent({ called_by: "bob" })] }));
    const owner = render(<RoomSection issue={planning()} me={alice} maxChars={ROOM_MAX_CHARS} />);
    fireEvent.click(await within(await screen.findByRole("region", { name: "Agents" })).findByRole("button", { name: "Dismiss alice-agent" }));
    await waitFor(() => expect(calls("DELETE", "/api/room/agents")).toEqual([{ repo: REPO, number: 122, branch: BRANCH, handle: "alice-agent" }]));
    expect(await screen.findByRole("status")).toHaveTextContent("alice-agent left the room.");
    owner.unmount();
    const caller = render(<RoomSection issue={planning()} me={{ login: "Bob" }} maxChars={ROOM_MAX_CHARS} />);
    expect(await within(await screen.findByRole("region", { name: "Agents" })).findByRole("button", { name: "Dismiss alice-agent" })).toBeInTheDocument();
    caller.unmount();
    render(<RoomSection issue={planning()} me={carol} maxChars={ROOM_MAX_CHARS} />);
    await within(await screen.findByRole("region", { name: "Agents" })).findByText("alice-agent");
    expect(screen.queryByRole("button", { name: "Dismiss alice-agent" })).toBeNull();
  });
});

describe("addressing an agent, and its reply under the turn", () => {
  it("names the handle of each agent in the room next to the box; none when no agent is in", async () => {
    serve(() => view({ agents: [agent(), agent({ handle: "carol-agent", owner: "carol", launch_id: "l2", left_at: minutesAgo(1), left_reason: "dismissed" })] }));
    const { unmount } = render(<RoomSection issue={planning()} me={bob} maxChars={ROOM_MAX_CHARS} />);
    const hint = await screen.findByTestId("room-address-hint");
    expect(hint).toHaveTextContent("@alice-agent");
    expect(hint).not.toHaveTextContent("@carol-agent");
    unmount();
    vi.restoreAllMocks();
    // Only an agent that left: nobody to address.
    serve(() => view({ agents: [agent({ left_at: minutesAgo(1), left_reason: "session_ended" })] }));
    render(<RoomSection issue={planning()} me={bob} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("No messages yet.");
    expect(screen.queryByTestId("room-address-hint")).toBeNull();
  });

  it("marks a message that addresses an agent, and shows the agent's reply under the turn it answers", async () => {
    serve(() =>
      view({
        agents: [agent()],
        messages: [
          message("m1", { text: "@alice-agent what does the seam carry?", addressed: ["alice-agent"], turn_id: "t1" }),
          message("m2", { author: "carol", text: "meanwhile, a note" }),
          message("m3", { author: "alice-agent", author_kind: "agent", text: "It carries the plan bundle.", turn_id: "t1" }),
          message("m4", { author: "alice-agent", author_kind: "agent", text: "An answer to a turn not shown.", turn_id: "t-gone" }),
        ],
      }),
    );
    render(<RoomSection issue={planning()} me={bob} maxChars={ROOM_MAX_CHARS} />);
    const log = await screen.findByRole("list", { name: "Messages in the room" });
    await within(log).findByText("It carries the plan bundle.");
    const top = [...log.children].map((li) => li.getAttribute("data-testid"));
    expect(top).toEqual(["room-message-m1", "room-message-m2", "room-message-m4"]);
    const turn = screen.getByTestId("room-message-m1");
    expect(turn).toHaveTextContent("to @alice-agent");
    const replies = within(turn).getByRole("list", { name: "Replies to this message" });
    expect(within(replies).getByTestId("room-message-m3")).toHaveClass("agent");
    expect(screen.getByTestId("room-message-m2")).not.toHaveTextContent("to @");
  });
});
