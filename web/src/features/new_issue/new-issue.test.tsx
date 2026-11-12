import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import type { NewIssueRequest } from "../../protocol.ts";
import { NewIssueForm } from "./NewIssueForm.tsx";

afterEach(() => vi.restoreAllMocks());

function fill(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

function renderForm(can = true, onCreated = vi.fn(), onNotice = vi.fn()) {
  render(<NewIssueForm overview={overview({ can_create_issues: can })} target={{ kind: "seam", id: "web-server" }} onCreated={onCreated} onCancel={() => {}} onNotice={onNotice} />);
  return { onCreated, onNotice };
}

function fillRequired() {
  fill("Title", "Poll less");
  fill("Summary", "The poll costs too many calls.");
  fill("What the change does", "- Lists branches once\nReads plans on a new head\n\n");
  fill("How we'll know it works", "- [ ] The call-order test asserts fewer calls");
  fill("What it must not break", "Progress latency");
}

describe("New issue here (ADR 0025)", () => {
  it("sends the sections as lines, to the owner's repo, pinned to the seam it started on", async () => {
    const fetch = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({ state: "created", issue: { repo: "m0x6A/rubato", number: 501 }, url: "u" }), { status: 201 }));
    const { onCreated, onNotice } = renderForm();
    const create = screen.getByRole("button", { name: "Create in m0x6A/rubato" });
    expect(create).toBeDisabled();
    expect(screen.getByText(/Still needed: a title/)).toBeVisible();
    fillRequired();
    expect(create).toBeEnabled();
    fireEvent.click(create);
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith({ repo: "m0x6A/rubato", number: 501 }));
    const body = JSON.parse(fetch.mock.calls[0]?.[1]?.body as string) as NewIssueRequest;
    expect(body.target).toEqual({ kind: "seam", id: "web-server" });
    expect(body.repo).toBe("m0x6A/rubato");
    expect(body.does).toEqual(["Lists branches once", "Reads plans on a new head"]);
    expect(body.know).toEqual(["The call-order test asserts fewer calls"]);
    expect(body.must_not_break).toEqual(["Progress latency"]);
    expect(body.request_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(onNotice).toHaveBeenCalledWith("Created m0x6A/rubato#501, pinned to seam web-server.");
  });

  it("keeps one request id for the draft, so a second press makes no second issue", async () => {
    const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(JSON.stringify({ state: "opening", message: "Still opening: GitHub has not confirmed this issue yet." }), { status: 202 }));
    renderForm();
    fillRequired();
    const create = screen.getByRole("button", { name: "Create in m0x6A/rubato" });
    fireEvent.click(create);
    await screen.findByText(/Still opening/);
    fireEvent.click(create);
    await waitFor(() => expect(fetch).toHaveBeenCalledTimes(2));
    const ids = fetch.mock.calls.map((c) => (JSON.parse(c[1]?.body as string) as NewIssueRequest).request_id);
    expect(ids[0]).toBe(ids[1]);
  });

  it("shows a failure the server explains", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({ state: "failed", message: "This request failed: 403." }), { status: 502 }));
    renderForm();
    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Create in m0x6A/rubato" }));
    expect(await screen.findByTestId("new-issue-message")).toHaveTextContent("This request failed: 403.");
  });

  it("says why it cannot create, and how to place the issue by hand", () => {
    const fetch = vi.spyOn(globalThis, "fetch");
    renderForm(false);
    expect(screen.getByTestId("new-issue-unavailable")).toHaveTextContent("Map: seam web-server");
    fillRequired();
    expect(screen.getByRole("button", { name: "Create in m0x6A/rubato" })).toBeDisabled();
    expect(fetch).not.toHaveBeenCalled();
  });
});
