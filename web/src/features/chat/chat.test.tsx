import { act, configure, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Me, SessionState, WebEvent } from "../../protocol.ts";
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

const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status });

/** A server holding `list`; a POST answers with `post`, by default the new message queued. */
function serve(list: LaunchMessages | (() => LaunchMessages), post: (body: unknown) => Response | Promise<Response> = (b) => json({ ...msg("queued"), id: "m-new", text: (b as { text: string }).text })) {
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (_input, init) => {
    if ((init?.method ?? "GET") === "GET") {
      return json(typeof list === "function" ? list() : list);
    }
    return post(JSON.parse(init?.body as string));
  });
  const calls = (method: string) => fetch.mock.calls.filter((c) => (c[1]?.method ?? "GET") === method);
  return { fetch, gets: () => calls("GET"), posts: () => calls("POST") };
}

const compose = () => screen.getByLabelText("Message to your session");
const openChat = () => fireEvent.click(screen.getByRole("button", { name: /Messages/ }));

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
    expect(screen.getByRole("button", { name: /Messages/ })).toHaveAttribute("aria-expanded", "false");
    expect(gets()).toHaveLength(0);
    openChat();
    await waitFor(() => expect(gets()).toHaveLength(1));
    unmount();
    render(<SessionChat session={session()} me={owner} />);
    expect(screen.getByRole("button", { name: /Messages/ })).toHaveAttribute("aria-expanded", "true");
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
    vi.spyOn(globalThis, "fetch").mockResolvedValue(json({ error: "unknown api path" }, 404));
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
    vi.spyOn(globalThis, "fetch").mockResolvedValue(json({ reason: "unknown_launch" }, 404));
    render(<SessionChat session={session()} me={owner} />);
    expect(await screen.findByText(/Messages could not be read: .*unknown_launch/)).toBeInTheDocument();
    expect(screen.queryByLabelText("Message to your session")).toBeNull();
  });
});
