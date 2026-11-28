import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Me } from "../../protocol.ts";
import { AccountMenu } from "./AccountMenu.tsx";

const ADA: Me = { login: "ada", tenants: [{ id: "acme", name: "Acme", role: "admin" }] };
const TWO: Me = { login: "ada", tenants: [...(ADA.tenants ?? []), { id: "bcorp", name: "Bcorp", role: "member" }] };

const open = () => fireEvent.click(screen.getByRole("button", { name: "Account ada" }));

afterEach(() => vi.unstubAllGlobals());

describe("the account menu (issue #357; tenants ADR 0087)", () => {
  it("a person in one tenant has no tenant to choose; Members and Log out are there, Operator is not", () => {
    render(<AccountMenu me={ADA} tenant="acme" onSwitchTenant={() => {}} onMembers={() => {}} onNotice={() => {}} />);
    open();
    expect(screen.queryByRole("menuitemradio")).toBeNull();
    expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual(["Members", "Log out"]);
  });

  it("lists every tenant the person is in, marks the one shown and switches", () => {
    const onSwitchTenant = vi.fn();
    render(<AccountMenu me={TWO} tenant="bcorp" onSwitchTenant={onSwitchTenant} onNotice={() => {}} />);
    open();
    const choices = screen.getAllByRole("menuitemradio");
    expect(choices.map((c) => c.textContent)).toEqual(["Acme", "Bcorpshowing"]);
    expect(choices[1]).toHaveAttribute("aria-checked", "true");
    fireEvent.click(choices[0]!);
    expect(onSwitchTenant).toHaveBeenCalledWith("acme");
  });

  it("an operator reaches the operator's page; a person in no tenant has no Members", () => {
    const onOperator = vi.fn();
    render(<AccountMenu me={{ login: "ada", operator: true, tenants: [] }} tenant={null} onSwitchTenant={() => {}} onOperator={onOperator} onNotice={() => {}} />);
    open();
    expect(screen.queryByRole("menuitem", { name: "Members" })).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: "Operator" }));
    expect(onOperator).toHaveBeenCalledTimes(1);
  });

  it("Log out ends the session on the server and starts over", async () => {
    const calls: { method: string; url: string }[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        calls.push({ method: init?.method ?? "GET", url });
        return new Response(null, { status: 204 });
      }),
    );
    const assign = vi.fn();
    vi.stubGlobal("location", { ...location, assign });
    render(<AccountMenu me={ADA} tenant="acme" onSwitchTenant={() => {}} onNotice={() => {}} />);
    open();
    fireEvent.click(screen.getByRole("menuitem", { name: "Log out" }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("/"));
    expect(calls).toEqual([{ method: "POST", url: "/auth/logout" }]);
  });

  it("a log out that fails says so and stays", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("boom", { status: 500 })));
    const onNotice = vi.fn();
    render(<AccountMenu me={ADA} tenant="acme" onSwitchTenant={() => {}} onNotice={onNotice} />);
    open();
    fireEvent.click(screen.getByRole("menuitem", { name: "Log out" }));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith(expect.stringContaining("You were not logged out")));
  });
});
