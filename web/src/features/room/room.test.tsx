import { act, configure, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { issue, overview } from "../../../test/fixtures.ts";
import type { IssueSummary, Me, Overview, RoomMessage, RoomView } from "../../protocol.ts";
import { emitWebEvent } from "../live/index.ts";
import { ROOM_MAX_CHARS, RoomSection, roomFor, roomLimit } from "./index.ts";

afterEach(() => vi.restoreAllMocks());
// As in the chat tests: a first render can pass 1 s while every file runs at once.
configure({ asyncUtilTimeout: 5000 });

const REPO = "m0x6A/rubato";
const BRANCH = "issue/122-plan-in-rubato";
const ROOM_URL = `/api/room?repo=${encodeURIComponent(REPO)}&number=122&branch=${encodeURIComponent(BRANCH)}`;
const now = new Date().toISOString();
const me: Me = { login: "bob" };

const draft = { draft_id: "d0000000-0000-4000-8000-000000000001", branch: BRANCH, owner: "alice", rev: 2, at: now, plan_sha: "p", bundle_hash: "h", live: true };
const drafted = (extra: Partial<IssueSummary> = {}): IssueSummary => issue(122, "web", { draft, ...extra });

function message(id: string, extra: Partial<RoomMessage> = {}): RoomMessage {
  return { id, author: "alice", author_kind: "person", text: `text ${id}`, at: now, ...extra };
}

const view = (messages: RoomMessage[], branch = BRANCH): RoomView => ({ repo: REPO, number: 122, branch, messages });
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

/** A server holding the room; a post answers with `post`, by default the new message, 201. */
function serve(room: () => RoomView | Response, post: (body: Record<string, unknown>) => Response | Promise<Response> = (b) => json(message("m-new", { author: "bob", text: String(b["text"]) }), 201)) {
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (_input, init) => {
    if ((init?.method ?? "GET") === "GET") {
      const r = room();
      return r instanceof Response ? r : json(r);
    }
    return post(JSON.parse(init?.body as string) as Record<string, unknown>);
  });
  const calls = (method: string) => fetch.mock.calls.filter((c) => (c[1]?.method ?? "GET") === method);
  return { gets: () => calls("GET"), posts: () => calls("POST") };
}

const log = () => screen.getByRole("list", { name: "Messages in the room" });
const box = () => screen.getByLabelText("Message to the room");

describe("which room an issue has (ADR 0057)", () => {
  it("the draft's branch first, else the plan's branch, else none", () => {
    expect(roomFor(drafted({ plan: { issue: { repo: REPO, number: 122 }, path: "docs/plans/122.md", affected_parts: [], git_ref: "issue/122-old" } }))).toEqual({ repo: REPO, number: 122, branch: BRANCH });
    expect(roomFor(issue(122, "web", { plan: { issue: { repo: REPO, number: 122 }, path: "docs/plans/122.md", affected_parts: [], git_ref: "issue/122-old" } }))).toEqual({ repo: REPO, number: 122, branch: "issue/122-old" });
    expect(roomFor(issue(122, "web"))).toBeNull();
    expect(roomFor(issue(122, "web", { plan: { issue: { repo: REPO, number: 122 }, path: "docs/plans/122.md", affected_parts: [] } }))).toBeNull();
  });

  it("no room, and nothing read, for an issue with neither draft nor plan", () => {
    const fetch = vi.spyOn(globalThis, "fetch");
    const { container } = render(<RoomSection issue={issue(122, "web")} me={me} maxChars={ROOM_MAX_CHARS} />);
    expect(container).toBeEmptyDOMElement();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("the length limit comes from the overview's limits when it carries them, else 8000", () => {
    expect(ROOM_MAX_CHARS).toBe(8000);
    expect(roomLimit(overview())).toBe(8000);
    expect(roomLimit({ ...overview(), limits: { room_message_max_chars: 120 } } as Overview)).toBe(120);
  });
});

describe("the room's messages, for the whole team", () => {
  it("reads the room from the team route and shows its messages oldest first, each with its author and time", async () => {
    const { gets } = serve(() => view([message("m1", { text: "first" }), message("m2", { author: "carol", text: "second\nwith two lines" })]));
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("first");
    expect(gets()[0]?.[0]).toBe(ROOM_URL);
    const region = screen.getByRole("region", { name: /Room/ });
    expect(region).toHaveTextContent(BRANCH);
    const items = within(log()).getAllByRole("listitem");
    expect(items.map((li) => li.querySelector(".room-text")?.textContent)).toEqual(["first", "second\nwith two lines"]);
    expect(items.map((li) => li.querySelector(".room-author")?.textContent)).toEqual(["alice", "carol"]);
    expect(items[0]?.querySelector("time")).toHaveAttribute("datetime", now);
  });

  it("a system seam notice reads as Rubato's and links the other issue and the seam", async () => {
    const notice = message("n1", { author: "rubato", author_kind: "system", text: "This plan and m0x6A/rubato#80 both touch seam web-server.", notice: { seam: "web-server", other: { repo: REPO, number: 80 }, other_branch: "issue/80-plans" } });
    serve(() => view([notice, message("n2", { author: "rubato", author_kind: "system", text: "again", notice: { seam: "companion-server", other: { repo: "acme/pantry", number: 7 }, other_branch: "issue/7-x" } })]));
    const onOpenIssue = vi.fn();
    const onShowSeam = vi.fn();
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} onOpenIssue={onOpenIssue} canOpen={(i) => i.number === 80} onShowSeam={onShowSeam} />);
    const item = (await screen.findByText(/both touch seam web-server/)).closest("li") as HTMLElement;
    expect(item).toHaveClass("system");
    expect(item.querySelector(".room-author")?.textContent).toBe("Rubato");
    expect(item).toHaveTextContent("notice");
    fireEvent.click(within(item).getByRole("button", { name: "m0x6A/rubato#80" }));
    expect(onOpenIssue).toHaveBeenCalledWith({ repo: REPO, number: 80 });
    fireEvent.click(within(item).getByRole("button", { name: "seam web-server" }));
    expect(onShowSeam).toHaveBeenCalledWith("web-server");
    expect(item).toHaveTextContent("issue/80-plans");
    // An issue this page does not track links to GitHub instead.
    const other = screen.getByText("again").closest("li") as HTMLElement;
    expect(within(other).getByRole("link", { name: "acme/pantry#7" })).toHaveAttribute("href", "https://github.com/acme/pantry/issues/7");
  });

  it("keeps an agent's message apart from a person's", async () => {
    serve(() => view([message("p1"), message("a1", { author: "alice-agent", author_kind: "agent", text: "from the agent" })]));
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    const agent = (await screen.findByText("from the agent")).closest("li") as HTMLElement;
    expect(agent).toHaveClass("agent");
    expect(agent).toHaveTextContent("agent");
    expect(agent.querySelector(".room-author")?.textContent).toBe("alice-agent");
    const person = screen.getByText("text p1").closest("li") as HTMLElement;
    expect(person).toHaveClass("person");
    expect(person.querySelector(".room-kind")).toBeNull();
  });

  it("says when the room is empty, and when it cannot be read", async () => {
    serve(() => view([]));
    const first = render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    expect(await screen.findByText("No messages yet.")).toBeInTheDocument();
    first.unmount();
    vi.restoreAllMocks();
    serve(() => json({ reason: "no_plan" }, 404));
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    expect(await screen.findByText("The room could not be read: This branch has no plan or draft, so it has no room (no_plan)")).toBeInTheDocument();
  });
});

describe("posting", () => {
  it("a post appears in the list, the box clears and focus stays there; the body names the room", async () => {
    const { posts } = serve(() => view([message("m1")]));
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("text m1");
    fireEvent.change(box(), { target: { value: "what about\nthe seam?" } });
    fireEvent.click(screen.getByRole("button", { name: "Post" }));
    // In the list: the box itself holds the same text until it clears.
    const mine = (await within(log()).findByText("what about\nthe seam?", { normalizer: (s) => s })).closest("li") as HTMLElement;
    expect(mine.querySelector(".room-author")?.textContent).toBe("bob");
    expect(JSON.parse(posts()[0]?.[1]?.body as string)).toEqual({ repo: REPO, number: 122, branch: BRANCH, text: "what about\nthe seam?" });
    expect(posts()[0]?.[0]).toBe("/api/room/messages");
    await waitFor(() => expect(box()).toHaveValue(""));
    expect(box()).toHaveFocus();
    expect(screen.getByRole("status")).toHaveTextContent("Posted");
    expect(within(log()).getAllByRole("listitem")).toHaveLength(2);
  });

  it("one send at a time; Ctrl+Enter and Cmd+Enter post, Enter is a newline", async () => {
    let release: (r: Response) => void = () => {};
    const { posts } = serve(
      () => view([]),
      () => new Promise<Response>((r) => (release = r)),
    );
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("No messages yet.");
    fireEvent.change(box(), { target: { value: "one" } });
    // Two presses inside one render: the second finds a post already out.
    act(() => {
      box().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
      box().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }));
    });
    expect(await screen.findByRole("button", { name: "Posting…" })).toBeDisabled();
    fireEvent.keyDown(box(), { key: "Enter", metaKey: true });
    expect(posts()).toHaveLength(1);
    await act(async () => release(json(message("m-one", { author: "bob", text: "one" }), 201)));
    await waitFor(() => expect(box()).toHaveValue(""));
    fireEvent.change(box(), { target: { value: "two" } });
    fireEvent.keyDown(box(), { key: "Enter" });
    expect(posts()).toHaveLength(1);
    fireEvent.keyDown(box(), { key: "Enter", metaKey: true });
    await waitFor(() => expect(posts()).toHaveLength(2));
  });

  it("a refusal says why in words with its code, and keeps the text", async () => {
    serve(
      () => view([]),
      () => json({ reason: "too_long" }, 422),
    );
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("No messages yet.");
    fireEvent.change(box(), { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: "Post" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Not posted: The message is longer than the room takes (too_long)");
    expect(box()).toHaveValue("hello");
    expect(screen.queryByRole("list", { name: "Messages in the room" })).toBeNull();
  });

  it("over the length limit the post waits and says by how much; a blank one cannot be posted", async () => {
    const { posts } = serve(() => view([]));
    render(<RoomSection issue={drafted()} me={me} maxChars={10} />);
    await screen.findByText("No messages yet.");
    expect(screen.getByRole("button", { name: "Post" })).toBeDisabled();
    fireEvent.change(box(), { target: { value: "12345678901" } });
    expect(screen.getByText("1 character over the limit of 10")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Post" })).toBeDisabled();
    fireEvent.keyDown(box(), { key: "Enter", ctrlKey: true });
    // Characters, as the server counts them: one emoji is one.
    fireEvent.change(box(), { target: { value: "😀😀😀😀😀😀😀😀😀😀" } });
    expect(screen.getByRole("button", { name: "Post" })).toBeEnabled();
    fireEvent.change(box(), { target: { value: "   " } });
    expect(screen.getByRole("button", { name: "Post" })).toBeDisabled();
    expect(posts()).toHaveLength(0);
  });

  it("someone not logged in reads the room but has no compose box", async () => {
    serve(() => view([message("m1")]));
    render(<RoomSection issue={drafted()} me={null} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("text m1");
    expect(screen.queryByLabelText("Message to the room")).toBeNull();
    expect(screen.getByText("Log in to post in the room.")).toBeInTheDocument();
  });
});

describe("kept current over /ws/web", () => {
  it("an updated event for this room reads it again; one for another branch, another issue or a draft does not", async () => {
    let messages = [message("m1")];
    const { gets } = serve(() => view(messages));
    render(<RoomSection issue={drafted()} me={me} maxChars={ROOM_MAX_CHARS} />);
    await screen.findByText("text m1");
    expect(gets()).toHaveLength(1);
    act(() => {
      emitWebEvent({ type: "updated", at: now, resource: { kind: "room", repo: REPO, number: 122, branch: "issue/122-other" } });
      emitWebEvent({ type: "updated", at: now, resource: { kind: "room", repo: REPO, number: 80, branch: BRANCH } });
      emitWebEvent({ type: "updated", at: now, resource: { kind: "draft", draft_id: draft.draft_id } });
    });
    expect(gets()).toHaveLength(1);
    messages = [...messages, message("m2", { text: "a reply" })];
    act(() => emitWebEvent({ type: "updated", at: now, resource: { kind: "room", repo: REPO, number: 122, branch: BRANCH } }));
    expect(await screen.findByText("a reply")).toBeInTheDocument();
    expect(gets()).toHaveLength(2);
  });
});
