import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MemberView, TenantInfo } from "../../protocol.ts";
import { forgetTenant, learnTenants } from "../../shared/workspace.ts";
import { emitWebEvent } from "../live/index.ts";
import { MembersView } from "./MembersView.tsx";

const ACME: TenantInfo = { id: "acme", name: "Acme", role: "admin" };
const two: TenantInfo[] = [ACME, { id: "bcorp", name: "Bcorp", role: "member" }];

function member(login: string, github_id: number, extra: Partial<MemberView> = {}): MemberView {
  return { login, github_id, role: "member", invited_by: "ada", invited_at: "2026-11-28T00:00:00Z", joined: true, ...extra };
}

type Answer = { status: number; body?: unknown };

/** A server holding `members`; the writes answer as given. Returns the calls made. */
function serve(members: MemberView[], answers: { post?: Answer; put?: Answer; del?: Answer } = {}) {
  const calls: { method: string; url: string; body: unknown }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      calls.push({ method, url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
      const answer = ({ status, body }: Answer) => new Response(body === undefined ? null : JSON.stringify(body), { status });
      switch (method) {
        case "POST":
          return answer(answers.post ?? { status: 201, body: member("Carol", 5, { joined: false }) });
        case "PUT":
          return answer(answers.put ?? { status: 204 });
        case "DELETE":
          return answer(answers.del ?? { status: 204 });
        default:
          return answer({ status: 200, body: members });
      }
    }),
  );
  return calls;
}

afterEach(() => {
  vi.unstubAllGlobals();
  forgetTenant();
});

describe("the members of a tenant (ADR 0088)", () => {
  const people = [member("ada", 1, { role: "admin" }), member("mia", 3), member("Carol", 5, { joined: false })];

  it("shows a member who is in the tenant, and nothing to change", async () => {
    learnTenants(["acme"]);
    const calls = serve(people);
    render(<MembersView tenant={{ ...ACME, role: "member" }} me="mia" onClose={() => {}} onNotice={() => {}} />);
    const list = await screen.findByTestId("member-list");
    expect(within(list).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["adaadmin", "mia (you)member", "Carol · invitedmember"]);
    expect(calls[0]).toMatchObject({ method: "GET", url: "/api/t/acme/members" });
    expect(screen.queryByTestId("invite-form")).toBeNull();
    expect(screen.queryByRole("button", { name: /Remove/ })).toBeNull();
    expect(screen.getByText("An admin of Acme invites and removes members.")).toBeInTheDocument();
  });

  it("lets an admin invite by login and says which account was invited", async () => {
    learnTenants(["acme"]);
    const calls = serve(people);
    const onNotice = vi.fn();
    render(<MembersView tenant={ACME} me="ada" onClose={() => {}} onNotice={onNotice} />);
    const form = await screen.findByTestId("invite-form");
    // The invite is the only gate: the form says what it opens.
    expect(form).toHaveTextContent("The invite is the only gate.");
    expect(form).toHaveTextContent("private ones included");
    const invite = within(form).getByRole("button", { name: "Invite" });
    expect(invite).toBeDisabled();
    fireEvent.change(within(form).getByLabelText("GitHub login"), { target: { value: " carol " } });
    fireEvent.click(invite);
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Carol (GitHub user 5) was invited to Acme as member"));
    expect(calls.find((c) => c.method === "POST")).toMatchObject({ url: "/api/t/acme/members", body: { login: "carol", role: "member" } });
  });

  it("says in words why an invite was refused, and keeps what was typed", async () => {
    learnTenants(["acme"]);
    serve(people, { post: { status: 422, body: { reason: "unknown_login" } } });
    render(<MembersView tenant={ACME} me="ada" onClose={() => {}} onNotice={() => {}} />);
    const form = await screen.findByTestId("invite-form");
    const input = within(form).getByLabelText("GitHub login") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "nobody" } });
    fireEvent.click(within(form).getByRole("button", { name: "Invite" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("GitHub has no account with that login.");
    expect(input.value).toBe("nobody");
  });

  it("removes only after a second press, and changes a role at once", async () => {
    learnTenants(["acme"]);
    const calls = serve(people);
    const onNotice = vi.fn();
    render(<MembersView tenant={ACME} me="ada" onClose={() => {}} onNotice={onNotice} />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove mia" }));
    expect(calls.some((c) => c.method === "DELETE")).toBe(false);
    fireEvent.click(screen.getByTestId("confirm-remove-mia"));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith("mia was removed from Acme"));
    expect(calls.find((c) => c.method === "DELETE")?.url).toBe("/api/t/acme/members/3");
    fireEvent.change(screen.getByRole("combobox", { name: "Role of mia" }), { target: { value: "admin" } });
    await waitFor(() => expect(calls.find((c) => c.method === "PUT")).toMatchObject({ url: "/api/t/acme/members/3", body: { role: "admin" } }));
  });

  it("says why the last admin stays", async () => {
    learnTenants(["acme"]);
    serve(people, { del: { status: 409, body: { reason: "last_admin" } } });
    render(<MembersView tenant={ACME} me="ada" onClose={() => {}} onNotice={() => {}} />);
    fireEvent.click(await screen.findByRole("button", { name: "Remove ada" }));
    fireEvent.click(screen.getByTestId("confirm-remove-ada"));
    expect(await screen.findByRole("alert")).toHaveTextContent("A tenant keeps at least one admin.");
  });

  it("reads the list again when the tenant's members changed elsewhere", async () => {
    learnTenants(["acme"]);
    const calls = serve(people);
    render(<MembersView tenant={ACME} me="ada" onClose={() => {}} onNotice={() => {}} />);
    await screen.findByTestId("member-list");
    const before = calls.filter((c) => c.method === "GET").length;
    emitWebEvent({ type: "changed", at: new Date().toISOString(), reason: "members", tenant: "acme" });
    await waitFor(() => expect(calls.filter((c) => c.method === "GET").length).toBe(before + 1));
    emitWebEvent({ type: "changed", at: new Date().toISOString(), reason: "pin", tenant: "acme" });
    expect(calls.filter((c) => c.method === "GET").length).toBe(before + 1);
  });
});
