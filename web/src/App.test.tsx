import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { issue, minutesAgo, openIssue, overview } from "../test/fixtures.ts";
import { App } from "./App.tsx";
import type { MapRef } from "./protocol.ts";
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
/** The team's stored layout, as `PUT` and `DELETE /api/layout` change it. */
let layoutStore: Record<string, { x: number; y: number }>;
let layoutAnswer: { status: number; body?: string };

beforeEach(() => {
  resetNeedsLogin();
  pinAnswer = { status: 201 };
  pinned = new Map();
  layoutStore = {};
  layoutAnswer = { status: 204 };
  vi.stubGlobal("WebSocket", QuietSocket);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/me") {
        return new Response(JSON.stringify({ login: "m0x6A" }), { status: 200 });
      }
      if (url === "/api/overview") {
        const open = [openIssue(902, "Backlog: an unpinned issue")].filter((i) => !pinned.has(i.issue.number));
        const tracked = [...pinned].map(([n, pins]) => issue(n, "", { pins, title: "Backlog: an unpinned issue" }));
        return new Response(JSON.stringify(overview({ issues: tracked, backlog: { issues: open, repos: [{ repo: REPO, listed_at: minutesAgo(1) }] }, layout: { positions: { ...layoutStore } } })), { status: 200 });
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
