import { act, configure, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LaunchLog, LogRole, Me, SessionLogEntry, SessionState, WebEvent } from "../../protocol.ts";
import { emitWebEvent } from "../live/index.ts";
import { SessionChat } from "./SessionChat.tsx";
import type { LaunchMessages, SessionMessageState, SessionMessageView } from "./types.ts";

afterEach(() => vi.restoreAllMocks());
// The first render of the file can take over a second while every file runs at once; the same
// 5 s the map tests give their slow waits, for every find and waitFor here (each file is isolated).
configure({ asyncUtilTimeout: 5000 });

const LAUNCH = "6f1c0c1e-0000-4000-8000-000000000001";
const OTHER = "6f1c0c1e-0000-4000-8000-000000000002";
const PATH = `/api/launches/${LAUNCH}/messages`;
const LOG_PATH = `/api/launches/${LAUNCH}/log`;
const now = new Date().toISOString();
const owner: Me = { login: "m0x6A" };
const colleague: Me = { login: "alice" };

const session = (extra: Partial<SessionState> = {}): SessionState => ({ companion_id: "c1", user: "m0x6A", cwd: "/w/122", launch_id: LAUNCH, source: "hook", status: "idle", last_seen: now, ...extra });

function msg(state: SessionMessageState, extra: Partial<SessionMessageView> = {}): SessionMessageView {
  return { id: `m-${state}`, launch_id: LAUNCH, kind: "chat", text: `text ${state}`, author: "m0x6A", state, created_at: now, ...extra };
}

function launch(extra: Partial<LaunchMessages> = {}): LaunchMessages {
  return { launch_id: LAUNCH, owner: "m0x6A", companion_can_receive: true, companion_live: true, messages: [], ...extra };
}

function entry(role: LogRole, prompt_id: string, extra: Partial<SessionLogEntry> = {}): SessionLogEntry {
  return { session_id: "s1", prompt_id, role, kind: role === "user" ? "prompt" : "reply", text: `${role} ${prompt_id}`, at: now, ...extra };
}

const transcript = (entries: SessionLogEntry[] = []): LaunchLog => ({ launch_id: LAUNCH, entries });

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

type Answer<T> = T | (() => T | Response);

/**
 * A server holding `list` and the launch `log`; a POST answers with `post`, by default the new
 * message queued. `gets` are the reads of the messages, `logGets` those of the log.
 */
function serve(list: Answer<LaunchMessages>, post: (body: unknown) => Response | Promise<Response> = (b) => json({ ...msg("queued"), id: "m-new", text: (b as { text: string }).text }), log: Answer<LaunchLog> = transcript()) {
  const answer = <T,>(a: Answer<T>) => {
    const v = typeof a === "function" ? (a as () => T | Response)() : a;
    return v instanceof Response ? v : json(v);
  };
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    if ((init?.method ?? "GET") === "GET") {
      return String(input).endsWith("/log") ? answer(log) : answer(list);
    }
    // A stop posts no body.
    return post(init?.body ? JSON.parse(init.body as string) : undefined);
  });
  const calls = (method: string, suffix: string) => fetch.mock.calls.filter((c) => (c[1]?.method ?? "GET") === method && String(c[0]).endsWith(suffix));
  return { fetch, gets: () => calls("GET", "/messages"), logGets: () => calls("GET", "/log"), posts: () => calls("POST", "/messages") };
}

const compose = () => screen.getByLabelText("Message to your session");
const openChat = () => fireEvent.click(screen.getByRole("button", { name: /Conversation/ }));

describe("who may write (ADR 0051: your own launched session only)", () => {
  it("the owner gets a compose box; the list reads from the team route, not a workspace one", async () => {
    const { gets } = serve(launch({ messages: [msg("delivered")] }));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("text delivered")).toBeInTheDocument();
    expect(compose()).toBeInTheDocument();
    expect(gets()[0]?.[0]).toBe(PATH);
  });

  it("compares the login without case", async () => {
    serve(launch({ owner: "m0x6A" }));
    render(<SessionChat session={session()} me={{ login: "M0X6A" }} />);
    expect(await screen.findByLabelText("Message to your session")).toBeInTheDocument();
  });

  it("a colleague sees the conversation but no compose box and no Send again", async () => {
    serve(launch({ messages: [msg("delivered"), msg("delivery_unknown")] }));
    render(<SessionChat session={session()} me={colleague} />);
    openChat();
    expect(await screen.findByText("text delivered")).toBeInTheDocument();
    expect(screen.getByText("delivery unknown")).toBeInTheDocument();
    expect(screen.queryByLabelText("Message to your session")).toBeNull();
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
    expect(screen.getByText("Only m0x6A can message this session.")).toBeInTheDocument();
  });

  it("someone else's session starts collapsed and reads nothing until opened; your own starts open", async () => {
    const { gets } = serve(launch());
    const { unmount } = render(<SessionChat session={session()} me={colleague} />);
    expect(screen.getByRole("button", { name: /Conversation/ })).toHaveAttribute("aria-expanded", "false");
    expect(gets()).toHaveLength(0);
    openChat();
    await waitFor(() => expect(gets()).toHaveLength(1));
    unmount();
    render(<SessionChat session={session()} me={owner} />);
    expect(screen.getByRole("button", { name: /Conversation/ })).toHaveAttribute("aria-expanded", "true");
    await waitFor(() => expect(gets()).toHaveLength(2));
  });

  it("a session Rubato did not launch has no messages at all", () => {
    const { fetch } = serve(launch());
    const { container } = render(<SessionChat session={session({ launch_id: null })} me={owner} />);
    expect(container).toBeEmptyDOMElement();
    expect(fetch).not.toHaveBeenCalled();
  });
});

describe("each state in words", () => {
  it("says what happened to every message", async () => {
    serve(
      launch({
        messages: [msg("queued"), msg("sent"), msg("delivered"), msg("refused", { detail: "not_paired_user" }), msg("failed", { detail: "tmux exited 1" }), msg("delivery_unknown")],
      }),
    );
    render(<SessionChat session={session()} me={owner} />);
    const words = async (id: string) => (await screen.findByTestId(`message-${id}`)).querySelector(".msg-state")?.textContent;
    expect(await words("m-queued")).toBe("waiting for the turn to end");
    expect(await words("m-sent")).toBe("sending…");
    expect(await words("m-delivered")).toBe("delivered");
    expect(await words("m-refused")).toBe("refused: not_paired_user");
    expect(await words("m-failed")).toBe("failed: tmux exited 1");
    expect(await words("m-delivery_unknown")).toBe("delivery unknown");
  });

  it("a queued message waits for the companion when the companion is away", async () => {
    serve(launch({ companion_live: false, messages: [msg("queued")] }));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("waiting for the companion")).toBeInTheDocument();
  });

  it("keeps the newlines of a message", async () => {
    serve(launch({ messages: [msg("delivered", { text: "line one\nline two" })] }));
    render(<SessionChat session={session()} me={owner} />);
    const text = (await screen.findByTestId("message-m-delivered")).querySelector(".chat-text");
    expect(text?.textContent).toBe("line one\nline two");
  });

  it("says when there are no messages yet", async () => {
    serve(launch());
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("No messages yet.")).toBeInTheDocument();
  });
});

describe("sending", () => {
  it("Send posts the text with its newlines intact, clears the box and reads the list again", async () => {
    const { gets, posts } = serve(launch());
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.change(await screen.findByLabelText("Message to your session"), { target: { value: "line one\nline two\n" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(gets()).toHaveLength(2));
    expect(posts()).toHaveLength(1);
    const [url, init] = posts()[0] as [string, RequestInit];
    expect(url).toBe(PATH);
    expect(JSON.parse(init.body as string)).toEqual({ text: "line one\nline two\n" });
    await waitFor(() => expect(compose()).toHaveValue(""));
    expect(compose()).toHaveFocus();
    expect(screen.getByRole("status")).toHaveTextContent("waiting for the turn to end");
  });

  it("Ctrl+Enter and Cmd+Enter send; the button is disabled while sending", async () => {
    let release: (r: Response) => void = () => {};
    const { posts } = serve(launch(), () => new Promise<Response>((r) => (release = r)));
    render(<SessionChat session={session()} me={owner} />);
    const box = await screen.findByLabelText("Message to your session");
    fireEvent.change(box, { target: { value: "go" } });
    fireEvent.keyDown(box, { key: "Enter", ctrlKey: true });
    expect(await screen.findByRole("button", { name: "Sending…" })).toBeDisabled();
    // A second press while the first is out does not post twice.
    fireEvent.keyDown(box, { key: "Enter", ctrlKey: true });
    expect(posts()).toHaveLength(1);
    await act(async () => release(json({ ...msg("queued"), id: "m-new", text: "go" })));
    await waitFor(() => expect(box).toHaveValue(""));
    fireEvent.change(box, { target: { value: "again" } });
    fireEvent.keyDown(box, { key: "Enter", metaKey: true });
    await waitFor(() => expect(posts()).toHaveLength(2));
    // Enter alone is a newline, not a send.
    fireEvent.keyDown(box, { key: "Enter" });
    expect(posts()).toHaveLength(2);
  });

  it("an empty or blank message cannot be sent", async () => {
    const { posts } = serve(launch());
    render(<SessionChat session={session()} me={owner} />);
    const box = await screen.findByLabelText("Message to your session");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    fireEvent.change(box, { target: { value: "  \n " } });
    fireEvent.keyDown(box, { key: "Enter", ctrlKey: true });
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(posts()).toHaveLength(0);
  });

  it("a 403 shows not_your_session and keeps the text", async () => {
    serve(launch(), () => json({ reason: "not_your_session" }, 403));
    render(<SessionChat session={session()} me={owner} />);
    const box = await screen.findByLabelText("Message to your session");
    fireEvent.change(box, { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("not_your_session");
    expect(alert).toHaveTextContent("Only the person who launched this session can message it");
    expect(box).toHaveValue("hello");
  });

  it("a 422 says the message is too long", async () => {
    serve(launch(), () => json({ reason: "too_long" }, 422));
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.change(await screen.findByLabelText("Message to your session"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("too_long");
  });

  it("Send again posts a new message with the same text, never the old id", async () => {
    const { posts } = serve(launch({ messages: [msg("delivery_unknown", { id: "m-old", text: "run the tests\nthen stop" })] }));
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.click(await screen.findByRole("button", { name: "Send again" }));
    await waitFor(() => expect(posts()).toHaveLength(1));
    const [url, init] = posts()[0] as [string, RequestInit];
    expect(url).toBe(PATH);
    expect(JSON.parse(init.body as string)).toEqual({ text: "run the tests\nthen stop" });
  });

  it("only a delivery-unknown message offers Send again", async () => {
    serve(launch({ messages: [msg("queued"), msg("sent"), msg("delivered"), msg("refused"), msg("failed")] }));
    render(<SessionChat session={session()} me={owner} />);
    await screen.findByText("text delivered");
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
  });
});

describe("what a refusal says", () => {
  it("an empty message, in words with its code", async () => {
    serve(launch(), () => json({ reason: "empty" }, 422));
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.change(await screen.findByLabelText("Message to your session"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Not sent: The message is empty (empty)");
  });

  it("a reason the web does not know is shown as the server said it", async () => {
    serve(launch(), () => json({ reason: "companion_revoked" }, 409));
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.change(await screen.findByLabelText("Message to your session"), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Not sent: companion_revoked");
  });

  it("a server without the route says so in words, not as JSON", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => json({ error: "unknown api path" }, 404));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("Messages could not be read: unknown api path")).toBeInTheDocument();
  });
});

describe("kept current over /ws/web", () => {
  it("an updated event for this launch reads the list again; one for another launch, or a changed event, does not", async () => {
    let state: LaunchMessages = launch({ messages: [msg("queued")] });
    const { gets } = serve(() => state);
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("waiting for the turn to end")).toBeInTheDocument();
    expect(gets()).toHaveLength(1);

    const other: WebEvent = { type: "updated", at: now, resource: { kind: "launch_messages", launch_id: OTHER } };
    const changed: WebEvent = { type: "changed", at: now, reason: "hook" };
    act(() => {
      emitWebEvent(other);
      emitWebEvent(changed);
    });
    expect(gets()).toHaveLength(1);

    state = launch({ messages: [msg("queued", { state: "delivered" })] });
    act(() => emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_messages", launch_id: LAUNCH } }));
    expect(await screen.findByText("delivered")).toBeInTheDocument();
    expect(gets()).toHaveLength(2);
  });

  it("stops listening once closed", async () => {
    const { gets } = serve(launch());
    render(<SessionChat session={session()} me={owner} />);
    await waitFor(() => expect(gets()).toHaveLength(1));
    openChat();
    act(() => emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_messages", launch_id: LAUNCH } }));
    expect(gets()).toHaveLength(1);
  });
});

describe("when the session cannot take a message", () => {
  it("an older companion: upgrade it, no compose box", async () => {
    serve(launch({ companion_can_receive: false, messages: [msg("delivery_unknown")] }));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("Upgrade the companion to chat with this session")).toBeInTheDocument();
    expect(screen.queryByLabelText("Message to your session")).toBeNull();
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("hooks lost at a companion restart: relaunch, no compose box", async () => {
    serve(launch({ messages: [msg("delivery_unknown")] }));
    render(<SessionChat session={session({ hooks_lost: true })} me={owner} />);
    expect(await screen.findByText("The companion restarted and lost this session's hooks: relaunch to chat")).toBeInTheDocument();
    expect(screen.queryByLabelText("Message to your session")).toBeNull();
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("a list that cannot be read says why", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => json({ reason: "unknown_launch" }, 404));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText(/Messages could not be read: .*unknown_launch/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Message to your session")).toBeNull();
  });
});

describe("the session log (ADR 0052): what was said, for the whole team", () => {
  const thread = () => screen.getByRole("list", { name: "Prompts and replies in this session" });

  it("a colleague reads the prompts and replies in order, newest at the bottom, from the team route", async () => {
    const { logGets } = serve(launch(), undefined, transcript([entry("user", "p1", { text: "plan #122" }), entry("assistant", "p1", { text: "Here is the plan\nwith two lines" }), entry("user", "p2", { text: "go on" })]));
    render(<SessionChat session={session()} me={colleague} />);
    openChat();
    await screen.findByText("go on");
    const items = within(thread()).getAllByRole("listitem");
    expect(items.map((li) => li.querySelector(".log-text")?.textContent)).toEqual(["plan #122", "Here is the plan\nwith two lines", "go on"]);
    expect(logGets()[0]?.[0]).toBe(LOG_PATH);
    // Who said it, and when, in words a screen reader reads.
    expect(items[0]).toHaveTextContent("Prompt");
    expect(items[0]).toHaveTextContent("from the terminal");
    expect(items[1]).toHaveTextContent("Claude");
    expect(items[1]?.querySelector("time")).toHaveAttribute("datetime", now);
  });

  it("labels the design review and the discussion summary", async () => {
    serve(launch(), undefined, transcript([entry("assistant", "p1", { kind: "review", text: "Review: full — passed" }), entry("assistant", "p2", { kind: "summary", text: "Summary: we chose b" }), entry("assistant", "p3", { text: "done" })]));
    render(<SessionChat session={session()} me={owner} />);
    await screen.findByText("done");
    const kinds = within(thread())
      .getAllByRole("listitem")
      .map((li) => li.querySelector(".log-kind")?.textContent);
    expect(kinds).toEqual(["Design review", "Discussion summary", "Reply"]);
  });

  it("says when a text was cut on the machine, and when a prompt was typed together with text in the terminal", async () => {
    serve(launch(), undefined, transcript([entry("user", "p1", { text: "half typed and the message", mixed: true }), entry("assistant", "p1", { text: "a very long reply", truncated: true }), entry("assistant", "p2", { text: "whole" })]));
    render(<SessionChat session={session()} me={owner} />);
    await screen.findByText("whole");
    const [prompt, cut, whole] = within(thread()).getAllByRole("listitem");
    expect(prompt).toHaveTextContent("typed together with text already in the terminal");
    expect(cut).toHaveTextContent("(cut)");
    expect(whole).not.toHaveTextContent("(cut)");
    expect(whole).not.toHaveTextContent("typed together");
  });

  it("a prompt that came from a Rubato message is marked and linked to it, and the message says answered once its reply is in", async () => {
    let entries = [entry("user", "p1", { text: "text delivered", message_id: "m-delivered" })];
    const { logGets } = serve(launch({ messages: [msg("delivered", { author: "m0x6A" })] }), undefined, () => transcript(entries));
    render(<SessionChat session={session()} me={owner} />);
    const message = await screen.findByTestId("message-m-delivered");
    const prompt = await within(await screen.findByRole("list", { name: "Prompts and replies in this session" })).findByText("text delivered");
    const promptItem = prompt.closest("li") as HTMLElement;
    expect(promptItem).toHaveTextContent("from Rubato, sent by m0x6A");
    expect(within(promptItem).getByRole("link", { name: "the message" })).toHaveAttribute("href", `#${message.id}`);
    // Delivered, no reply yet: not answered, but it points to where it went.
    expect(message.querySelector(".msg-answered")).toBeNull();
    expect(within(message).getByRole("link", { name: "see it in the conversation" })).toHaveAttribute("href", `#${promptItem.id}`);

    entries = [...entries, entry("assistant", "p1", { text: "on it" })];
    act(() => emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_log", launch_id: LAUNCH } }));
    await within(thread()).findByText("on it");
    expect(logGets()).toHaveLength(2);
    expect(message.querySelector(".msg-answered")?.textContent).toBe("answered");
    const reply = within(thread()).getByText("on it").closest("li") as HTMLElement;
    expect(within(message).getByRole("link", { name: "see the reply" })).toHaveAttribute("href", `#${reply.id}`);
    // The targets take focus, so following the link moves the reader there.
    expect(reply).toHaveAttribute("tabindex", "-1");
    expect(message).toHaveAttribute("tabindex", "-1");
  });

  it("a reply answers only the prompt of its own turn and session", async () => {
    serve(launch({ messages: [msg("delivered")] }), undefined, transcript([entry("user", "p1", { message_id: "m-delivered" }), entry("assistant", "p2"), entry("assistant", "p1", { session_id: "s2" })]));
    render(<SessionChat session={session()} me={owner} />);
    await within(await screen.findByRole("list", { name: "Prompts and replies in this session" })).findByText("assistant p2");
    expect(screen.getByTestId("message-m-delivered").querySelector(".msg-answered")).toBeNull();
  });

  it("marks where a new Claude Code session continues the launch", async () => {
    serve(launch(), undefined, transcript([entry("user", "p1"), entry("assistant", "p1"), entry("user", "p2", { session_id: "s2" })]));
    render(<SessionChat session={session()} me={owner} />);
    await screen.findByText("user p2");
    const items = within(thread()).getAllByRole("listitem");
    expect(items.map((li) => li.querySelector(".log-new-session") !== null)).toEqual([false, false, true]);
  });

  it("an updated launch_log event for this launch reads the log again; one for another launch, or for its messages, does not", async () => {
    const { gets, logGets } = serve(launch(), undefined, transcript([entry("user", "p1")]));
    render(<SessionChat session={session()} me={owner} />);
    await screen.findByText("user p1");
    expect(logGets()).toHaveLength(1);
    act(() => {
      emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_log", launch_id: OTHER } });
      emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_messages", launch_id: LAUNCH } });
    });
    expect(logGets()).toHaveLength(1);
    expect(gets()).toHaveLength(2);
    act(() => emitWebEvent({ type: "updated", at: now, resource: { kind: "launch_log", launch_id: LAUNCH } }));
    expect(logGets()).toHaveLength(2);
    expect(gets()).toHaveLength(2);
  });

  it("says when nothing was said yet, and when the log cannot be read", async () => {
    serve(launch(), undefined, transcript());
    const first = render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("Nothing has been said in this session yet.")).toBeInTheDocument();
    first.unmount();
    vi.restoreAllMocks();
    serve(launch(), undefined, () => json({ reason: "unknown_launch" }, 404));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText("The conversation could not be read: Rubato does not know this launch (unknown_launch)")).toBeInTheDocument();
    // The messages still show, and the owner can still write.
    expect(await screen.findByLabelText("Message to your session")).toBeInTheDocument();
  });
});

describe("stop your own session (ADR 0062)", () => {
  const stops = (fetch: ReturnType<typeof serve>["fetch"]) => fetch.mock.calls.filter((c) => c[1]?.method === "POST" && String(c[0]) === `/api/launches/${LAUNCH}/stop`);

  it("the owner stops only after confirming", async () => {
    const { fetch } = serve(launch(), () => json({ ...msg("sent"), id: "m-stop", kind: "stop", text: "Stopped by m0x6A" }));
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.click(await screen.findByRole("button", { name: "Stop this session" }));
    expect(stops(fetch)).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Really stop this session" }));
    await waitFor(() => expect(stops(fetch)).toHaveLength(1));
    expect(await screen.findByText(/Stop sent/)).toBeInTheDocument();
  });

  it("an offline companion is not offered Stop", async () => {
    serve(launch({ companion_live: false }));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByRole("button", { name: "Stop this session" })).toBeDisabled();
  });

  it("a refusal says why, in words and with its code", async () => {
    serve(launch(), () => json({ reason: "companion_offline" }, 409));
    render(<SessionChat session={session()} me={owner} />);
    fireEvent.click(await screen.findByRole("button", { name: "Stop this session" }));
    fireEvent.click(screen.getByRole("button", { name: "Really stop this session" }));
    expect(await screen.findByText(/Not stopped: The companion is not connected.*\(companion_offline\)/)).toBeInTheDocument();
  });

  it("a colleague sees no Stop", async () => {
    serve(launch());
    render(<SessionChat session={session()} me={colleague} />);
    openChat();
    await screen.findByText(/Only m0x6A can message/);
    expect(screen.queryByRole("button", { name: /stop this session/i })).toBeNull();
  });
});

describe("a message a lead agent sent (ADR 0072)", () => {
  it("is marked via lead agent, and a person's own message is not", async () => {
    serve(launch({ messages: [msg("delivered", { id: "m-lead", via_launch: "aaaaaaaa-0000-4000-8000-000000000001" }), msg("queued", { id: "m-own" })] }));
    // Your own session starts open.
    render(<SessionChat session={session()} me={owner} />);
    await screen.findByTestId("message-m-lead");
    expect(screen.getByTestId("via-lead-m-lead")).toHaveTextContent("via lead agent");
    expect(screen.queryByTestId("via-lead-m-own")).toBeNull();
  });
});
