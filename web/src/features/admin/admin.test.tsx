import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AdminInstallation, AdminTenant, PurgeReport } from "../../protocol.ts";
import { AdminView } from "./AdminView.tsx";
import { words } from "./api.ts";

const TENANTS: AdminTenant[] = [
  { id: "acme", name: "Acme", installations: [11], members: 2, admins: 1, workspaces: 1 },
  { id: "empty", name: "Empty", installations: [], members: 1, admins: 1, workspaces: 0 },
];
const INSTALLATIONS: AdminInstallation[] = [
  { installation_id: 11, account_login: "acme", account_type: "Organization", state: "attached", tenant: "acme" },
  { installation_id: 22, account_login: "bcorp", account_type: "Organization", state: "detached", tenant: "bcorp" },
  { installation_id: 33, account_login: "newco", account_type: "User", state: "unattached" },
];
const COUNTED: PurgeReport = { installation_id: 22, tenant: "bcorp", repos: ["bcorp/map", "bcorp/web"], rows: { plans: 2, room_messages: 5 }, purged: false };

type Answer = { status: number; body?: unknown };

function serve(answers: Record<string, Answer> = {}) {
  const calls: { method: string; url: string; body: unknown }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      calls.push({ method, url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
      const key = `${method} ${url}`;
      const fallback: Record<string, Answer> = {
        "GET /api/admin/tenants": { status: 200, body: TENANTS },
        "GET /api/admin/installations": { status: 200, body: INSTALLATIONS },
        "POST /api/admin/installations/22/purge": { status: 200, body: COUNTED },
        "POST /api/admin/installations/22/purge?confirm=true": { status: 200, body: { ...COUNTED, purged: true } },
        "POST /api/admin/tenants": { status: 201, body: { id: "newco", name: "Newco", installations: [], members: 1, admins: 1, workspaces: 0 } },
      };
      const { status, body } = answers[key] ?? fallback[key] ?? { status: 204 };
      return new Response(body === undefined ? null : JSON.stringify(body), { status });
    }),
  );
  return calls;
}

afterEach(() => vi.unstubAllGlobals());

describe("the operator's page (ADR 0087, 0088)", () => {
  it("shows tenants as names and counts, and installations with their state", async () => {
    serve();
    render(<AdminView onClose={() => {}} onNotice={() => {}} />);
    const acme = await screen.findByTestId("tenant-acme");
    expect(acme).toHaveTextContent("Acme");
    expect(acme).toHaveTextContent("2 members, 1 admin · 1 workspace · installation 11");
    expect(within(acme).getByRole("button", { name: "Delete" })).toBeDisabled();
    expect(within(screen.getByTestId("tenant-empty")).getByRole("button", { name: "Delete" })).toBeEnabled();
    expect(await screen.findByTestId("installation-11")).toHaveTextContent("attached · tenant acme");
    expect(screen.getByTestId("installation-22")).toHaveTextContent("detached, rows kept · tenant bcorp");
    expect(screen.getByTestId("installation-33")).toHaveTextContent("not attached");
  });

  it("creates a tenant with its first admin", async () => {
    const calls = serve();
    const onNotice = vi.fn();
    render(<AdminView onClose={() => {}} onNotice={onNotice} />);
    const form = await screen.findByTestId("new-tenant");
    const create = within(form).getByRole("button", { name: "Create tenant" });
    expect(create).toBeDisabled();
    fireEvent.change(within(form).getByLabelText("Id"), { target: { value: "newco" } });
    fireEvent.change(within(form).getByLabelText("Name"), { target: { value: "Newco" } });
    fireEvent.change(within(form).getByLabelText("GitHub login of its first admin"), { target: { value: "carol" } });
    fireEvent.click(create);
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Tenant Newco was created with carol as its admin"));
    expect(calls.find((c) => c.method === "POST" && c.url === "/api/admin/tenants")?.body).toEqual({ id: "newco", name: "Newco", first_admin: "carol", installations: [] });
  });

  it("attaches an installation to the tenant chosen, and says who holds one that is taken", async () => {
    const calls = serve({ "POST /api/admin/tenants/empty/installations/33": { status: 409, body: { reason: "installation_taken", detail: "newco" } } });
    render(<AdminView onClose={() => {}} onNotice={() => {}} />);
    const row = await screen.findByTestId("installation-33");
    const attach = within(row).getByRole("button", { name: "Attach" });
    expect(attach).toBeDisabled();
    fireEvent.change(within(row).getByRole("combobox"), { target: { value: "empty" } });
    fireEvent.click(attach);
    expect(await screen.findByText("That installation is attached to tenant newco. Detach it there first.")).toBeInTheDocument();
    expect(calls.some((c) => c.url === "/api/admin/tenants/empty/installations/33")).toBe(true);
  });

  it("purges only after showing what would be deleted", async () => {
    const calls = serve();
    const onNotice = vi.fn();
    render(<AdminView onClose={() => {}} onNotice={onNotice} />);
    const row = await screen.findByTestId("installation-22");
    fireEvent.click(within(row).getByRole("button", { name: "Purge…" }));
    const confirm = await screen.findByTestId("purge-confirm");
    expect(confirm).toHaveTextContent("Purging deletes 7 rows of 2 repositories in tenant bcorp: plans 2, room_messages 5. This cannot be undone.");
    expect(calls.some((c) => c.url.endsWith("?confirm=true"))).toBe(false);
    fireEvent.click(within(confirm).getByRole("button", { name: "Delete these rows" }));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith("Purged 7 rows of bcorp"));
    expect(calls.some((c) => c.url === "/api/admin/installations/22/purge?confirm=true")).toBe(true);
  });

  it("still shows the tenants when GitHub cannot list the installations", async () => {
    serve({ "GET /api/admin/installations": { status: 502, body: { reason: "lookup_failed", detail: "not acting as a GitHub App" } } });
    render(<AdminView onClose={() => {}} onNotice={() => {}} />);
    expect(await screen.findByTestId("tenant-acme")).toBeInTheDocument();
    expect(await screen.findByText("GitHub could not be asked. Nothing was changed. (not acting as a GitHub App)")).toBeInTheDocument();
  });

  it("has words for every reason the operator routes refuse with", () => {
    const reasons = ["bad_id", "duplicate", "id_has_rows", "unknown_login", "unknown_installation", "installation_taken", "rows_of_other_tenant", "installation_in_use", "tenant_not_empty", "not_detached", "last_admin", "lookup_failed", "not_operator"];
    for (const r of reasons) {
      expect(words(r), r).not.toBe(r);
    }
    expect(words("something_new")).toBe("something_new");
  });
});
