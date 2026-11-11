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

beforeEach(() => {
  resetNeedsLogin();
  pinAnswer = { status: 201 };
  pinned = new Map();
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
        return new Response(JSON.stringify(overview({ issues: tracked, backlog: { issues: open, repos: [{ repo: REPO, listed_at: minutesAgo(1) }] } })), { status: 200 });
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
