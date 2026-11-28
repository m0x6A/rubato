import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { issue, minutesAgo, openIssue, overview } from "../test/fixtures.ts";
import { App } from "./App.tsx";
import type { IssueSummary, MapRef } from "./protocol.ts";
import { resetNeedsLogin } from "./shared/api.ts";

class QuietSocket {
  onopen: (() => void) | null = null;
  onmessage: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  close() {}
}

const REPO = "m0x6A/rubato";
/** What `POST /api/pins` does in this test: a status and body, or "offline" for a fetch that rejects. */
let pinAnswer: { status: number; body?: string } | "offline";
let pinned: Map<number, MapRef[]>;
/** Tracked issues the overview holds besides the pinned ones. */
let others: IssueSummary[];
/** The session has ended: every API call answers 401. */
let loggedOut: boolean;
/** The team's stored layout, as `PUT` and `DELETE /api/layout` change it. */
let layoutStore: Record<string, { x: number; y: number }>;
let layoutAnswer: { status: number; body?: string };

beforeEach(() => {
  // Every test starts on the issue list: the view is in the URL (issue #357).
  history.replaceState(null, "", "/");
  resetNeedsLogin();
  pinAnswer = { status: 201 };
  pinned = new Map();
  others = [];
  loggedOut = false;
  layoutStore = {};
  layoutAnswer = { status: 204 };
  vi.stubGlobal("WebSocket", QuietSocket);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      if (loggedOut) {
        return new Response(JSON.stringify({ login_url: "/auth/github/login" }), { status: 401 });
      }
      if (url === "/api/me") {
        return new Response(JSON.stringify({ login: "m0x6A" }), { status: 200 });
      }
      if (url === "/api/overview") {
        const open = [openIssue(902, "Backlog: an unpinned issue")].filter((i) => !pinned.has(i.issue.number));
        const tracked = [...pinned].map(([n, pins]) => issue(n, "", { pins, title: "Backlog: an unpinned issue" }));
        return new Response(JSON.stringify(overview({ issues: [...tracked, ...others], backlog: { issues: open, repos: [{ repo: REPO, listed_at: minutesAgo(1) }] }, layout: { positions: { ...layoutStore } } })), { status: 200 });
      }
      if (url === "/api/layout" && init?.method === "DELETE") {
        const removed = { positions: layoutStore };
        layoutStore = {};
        return new Response(JSON.stringify(removed), { status: 200 });
      }
      if (url === "/api/layout" && init?.method === "PUT") {
        if (layoutAnswer.status >= 300) {
          return new Response(layoutAnswer.body ?? null, { status: layoutAnswer.status });
        }
        layoutStore = { ...layoutStore, ...(JSON.parse(String(init.body)) as { positions: typeof layoutStore }).positions };
        return new Response(null, { status: 204 });
      }
      if (url === "/api/pins" && init?.method === "POST") {
        if (pinAnswer === "offline") {
          throw new TypeError("Failed to fetch");
        }
        if (pinAnswer.status < 300) {
          const req = JSON.parse(String(init.body)) as { issue: { number: number }; target: MapRef };
          pinned.set(req.issue.number, [...(pinned.get(req.issue.number) ?? []), req.target]);
        }
        return new Response(pinAnswer.body ?? null, { status: pinAnswer.status });
      }
      return new Response("not found", { status: 404 });
    }),
  );
});
afterEach(() => vi.unstubAllGlobals());

async function pinFromTheMenu(target: RegExp) {
  render(<App />);
  const row = await screen.findByTestId("backlog-row-902");
  fireEvent.click(within(row).getByRole("button", { name: "Pin m0x6A/rubato#902 to…" }));
  fireEvent.click(screen.getByRole("menuitem", { name: target }));
}

describe("the shell (issue #357)", () => {
  it("the top bar holds the brand, the workspace menu, the live dot and the account menu, and nothing else", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /^Workspace / });
    const bar = screen.getByRole("banner");
    // The words beside the dot belong to it; they show only when the signal has stopped.
    const items = [...bar.querySelectorAll(".brand > *, .status > *:not([data-testid=\"live-words\"])")];
    console.log(`checked ${items.length} items in the top bar`);
    expect(items).toHaveLength(4);
    expect(within(bar).getByRole("link", { name: "Rubato" })).toHaveAttribute("href", "#/issues");
    expect(within(bar).getByTestId("live-dot")).toBeInTheDocument();
    expect(within(bar).getByRole("button", { name: "Account m0x6A" })).toBeInTheDocument();
  });

  it("Configure workspace is in the workspace menu with one workspace too, opens as a page with the map hidden, and closes to where the person was (issue #189)", async () => {
    history.replaceState(null, "", "/#/repos");
    const { container } = render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^Workspace / }));
    // One workspace: nothing to switch to, and the item is there all the same.
    expect(screen.queryByRole("menuitemradio")).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: "Configure workspace…" }));
    expect(location.hash).toBe("#/settings");

    const view = await screen.findByRole("region", { name: "Configure workspace" });
    expect(screen.getByTestId("body")).toHaveAttribute("data-page", "settings");
    // The map is hidden, not gone: it keeps its lens and positions.
    expect(container.querySelector("section.map")).toHaveAttribute("hidden");
    expect(screen.queryByRole("button", { name: /^Issues/ })).toBeNull();
    // This stub serves no settings: the view says so instead of drawing an empty form.
    expect(await within(view).findByRole("alert")).toHaveTextContent("The settings could not be read");

    fireEvent.click(within(view).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Configure workspace" })).toBeNull());
    expect(location.hash).toBe("#/repos");
    expect(screen.getByTestId("body")).not.toHaveAttribute("data-page");
    expect(container.querySelector("section.map")).not.toHaveAttribute("hidden");
  });

  it("the live dot says nothing while live and says so in words when the signal stops", async () => {
    render(<App />);
    const dot = await screen.findByTestId("live-dot");
    // The stub socket never opens: the page is not live, and must not look it.
    expect(dot).toHaveAccessibleName("disconnected");
    expect(await screen.findByTestId("live-words")).toHaveTextContent(/^disconnected/);
  });
});

describe("the view comes from the URL (issue #357)", () => {
  const at = (n: number) => ({ issue: { repo: REPO, number: n }, state: "designed" as const, last_seen: minutesAgo(1) });

  it("a link opens the issue it names, and Escape closes it to the list", async () => {
    others = [issue(1, "web", { progress: at(1) })];
    history.replaceState(null, "", "/#/issues/m0x6A/rubato/1");
    render(<App />);
    expect(await screen.findByTestId("issue-detail")).toHaveTextContent("m0x6A/rubato#1");
    expect(screen.queryByTestId("not-found")).toBeNull();

    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("issue-detail")).toBeNull());
    expect(location.hash).toBe("#/issues");
  });

  it("an issue Rubato does not track is said to be missing, with the list under it", async () => {
    others = [issue(1, "web", { progress: at(1) })];
    history.replaceState(null, "", "/#/issues/m0x6A/rubato/77");
    render(<App />);
    expect(await screen.findByTestId("not-found")).toHaveTextContent("m0x6A/rubato#77 is not among the issues Rubato tracks");
    expect(screen.queryByTestId("issue-detail")).toBeNull();
    expect(screen.getByTestId("issue-row-1")).toBeInTheDocument();

    fireEvent.click(within(screen.getByTestId("not-found")).getByRole("button", { name: "All issues" }));
    await waitFor(() => expect(screen.queryByTestId("not-found")).toBeNull());
  });

  it("a fragment that names no view says so", async () => {
    history.replaceState(null, "", "/#/nowhere");
    render(<App />);
    expect(await screen.findByTestId("not-found")).toHaveTextContent("Nothing is at #/nowhere.");
  });

  it("a tab is a view with an address, and opening an issue is a step Back can undo", async () => {
    others = [issue(1, "web", { progress: at(1) })];
    render(<App />);
    fireEvent.click(await screen.findByTestId("tab-repos"));
    expect(location.hash).toBe("#/repos");
    fireEvent.click(screen.getByRole("button", { name: /^Issues/ }));
    expect(location.hash).toBe("#/issues");
    const before = history.length;
    fireEvent.click(await screen.findByTestId("issue-row-1"));
    expect(location.hash).toBe("#/issues/m0x6A/rubato/1");
    expect(history.length).toBe(before + 1);
  });
});

describe("open on GitHub by default (#265)", () => {
  it("the head of the issue list counts what is not finished", async () => {
    const at = (n: number, state: "designed" | "done") => ({ issue: { repo: REPO, number: n }, state, last_seen: minutesAgo(1) });
    others = [issue(1, "web", { progress: at(1, "designed") }), issue(2, "web", { progress: at(2, "done") }), issue(3, "web", { progress: at(3, "designed"), closed: true, finished: true })];
    render(<App />);
    // The count sits where it means something: at the head of the list, not in the nav (issue #357).
    expect(await screen.findByTestId("team-track")).toHaveTextContent("1 tracked issue");
    expect(screen.getByRole("button", { name: "Issues" })).toBeInTheDocument();
    expect(screen.getByTestId("issues-view-open")).toHaveTextContent("Open (1)");
    expect(screen.getByTestId("issues-view-done")).toHaveTextContent("Done (2)");
  });
});

describe("without a session", () => {
  // The shell renders first and the 401 arrives after: every hook must run on both renders.
  it("the login page replaces the shell without an error", async () => {
    loggedOut = true;
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<App />);
    expect(await screen.findByRole("link", { name: "Log in with GitHub" })).toBeInTheDocument();
    expect(screen.queryByTestId("body")).toBeNull();
    expect(errors.mock.calls.map((c) => String(c[0])).filter((m) => /hooks/i.test(m))).toEqual([]);
    errors.mockRestore();
  });
});

describe("the panel's three sizes (#265)", () => {
  const body = () => screen.getByTestId("body");
  const map = () => document.querySelector("section.map")!;
  /** The window is wide or narrow: what `(min-width: 901px)` answers. */
  const windowIs = (wide: boolean) => vi.stubGlobal("matchMedia", () => ({ matches: wide, addEventListener() {}, removeEventListener() {} }));
  afterEach(() => localStorage.clear());

  it("one button cycles normal, wide, full and back, names what it does next, and the browser remembers", async () => {
    windowIs(true);
    const first = render(<App />);
    const button = await screen.findByTestId("panel-size");
    expect(body()).toHaveAttribute("data-panel", "normal");
    expect(button).toHaveAccessibleName("Widen panel");
    fireEvent.click(button);
    expect(body()).toHaveAttribute("data-panel", "wide");
    expect(map()).not.toHaveAttribute("hidden");
    expect(button).toHaveAccessibleName("Fill the window with the panel");
    fireEvent.click(button);
    expect(body()).toHaveAttribute("data-panel", "full");
    expect(map()).toHaveAttribute("hidden");
    expect(button).toHaveAccessibleName("Back to normal panel width");
    expect(localStorage.getItem("rubato.panel")).toBe("full");
    first.unmount();
    // A reload: the stored size is read back.
    render(<App />);
    await screen.findByTestId("panel-size");
    expect(body()).toHaveAttribute("data-panel", "full");
    fireEvent.click(screen.getByTestId("panel-size"));
    expect(body()).toHaveAttribute("data-panel", "normal");
    expect(map()).not.toHaveAttribute("hidden");
  });

  it("a stacked window has one size and no button, whatever is stored", async () => {
    localStorage.setItem("rubato.panel", "full");
    windowIs(false);
    render(<App />);
    await screen.findByRole("button", { name: /^Issues/ });
    expect(body()).toHaveAttribute("data-panel", "normal");
    expect(map()).not.toHaveAttribute("hidden");
    expect(screen.queryByTestId("panel-size")).toBeNull();
  });

  it("renders at normal when storage throws or holds something else", async () => {
    windowIs(true);
    localStorage.setItem("rubato.panel", "huge");
    const first = render(<App />);
    await screen.findByTestId("panel-size");
    expect(body()).toHaveAttribute("data-panel", "normal");
    first.unmount();
    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    render(<App />);
    fireEvent.click(await screen.findByTestId("panel-size"));
    expect(body()).toHaveAttribute("data-panel", "wide");
    get.mockRestore();
    set.mockRestore();
  });
});

describe("pinning from the shell (S3, N16)", () => {
  it("a pin is announced and focus lands on the issue's heading", async () => {
    await pinFromTheMenu(/^rubato-server\b/);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("m0x6A/rubato#902 pinned to rubato-server"));
    // The row left "Not pinned", taking the menu's button with it; focus must not fall to the body.
    const heading = await screen.findByRole("heading", { name: "m0x6A/rubato#902" });
    await waitFor(() => expect(heading).toHaveFocus());
    expect(document.activeElement).not.toBe(document.body);
    expect(screen.queryByTestId("backlog-row-902")).toBeNull();
  });

  it("the notice is dismissed with a real button", async () => {
    await pinFromTheMenu(/^rubato-server\b/);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("pinned to rubato-server"));
    // Not click-anywhere: clicking the text leaves it.
    fireEvent.click(screen.getByRole("status"));
    expect(screen.getByRole("status")).toHaveTextContent("pinned to rubato-server");
    const dismiss = screen.getByRole("button", { name: "Dismiss" });
    expect(dismiss.tagName).toBe("BUTTON");
    fireEvent.click(dismiss);
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
    expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
  });

  it("a refused pin shows the server's reason", async () => {
    pinAnswer = { status: 422, body: "rubato-server is a node, not a seam" };
    await pinFromTheMenu(/^rubato-server\b/);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("m0x6A/rubato#902 was not pinned to rubato-server: rubato-server is a node, not a seam"));
    expect(screen.getByTestId("backlog-row-902")).toBeInTheDocument();
  });

  it("a network failure says the server could not be reached", async () => {
    pinAnswer = "offline";
    await pinFromTheMenu(/^rubato-server\b/);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("m0x6A/rubato#902 was not pinned to rubato-server: Could not reach the server"));
    expect(screen.getByRole("status")).not.toHaveTextContent("Failed to fetch");
  });
});

describe("the shared layout from the shell (S7)", () => {
  const where = (id: string) => document.querySelector<HTMLElement>(`.react-flow__node[data-id="${id}"]`)?.style.transform;

  it("Auto-arrange then Undo restores", async () => {
    layoutStore = { web: { x: 1234, y: 567 }, "rubato-server": { x: 40, y: 900 } };
    render(<App />);
    await waitFor(() => expect(where("web")).toBe("translate(1234px,567px)"), { timeout: 5000 });

    fireEvent.click(screen.getByRole("button", { name: "Auto-arrange" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Auto-arranged for everyone: 2 saved positions removed"));
    expect(layoutStore).toEqual({});
    await waitFor(() => expect(where("web")).not.toBe("translate(1234px,567px)"));
    expect(screen.getByRole("button", { name: "Auto-arrange" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("The saved positions are back"));
    expect(layoutStore).toEqual({ web: { x: 1234, y: 567 }, "rubato-server": { x: 40, y: 900 } });
    await waitFor(() => expect(where("web")).toBe("translate(1234px,567px)"));
    expect(where("rubato-server")).toBe("translate(40px,900px)");
    // Undo is offered once.
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
  });

  it("a restore the server refuses says so and offers no second Undo", async () => {
    layoutStore = { web: { x: 1, y: 2 } };
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Auto-arrange" }, { timeout: 5000 }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Undo" })).toBeInTheDocument());
    layoutAnswer = { status: 422, body: "web is not a part in map v2" };
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("The saved positions were not restored: web is not a part in map v2"));
  });
});
