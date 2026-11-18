import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import type { ObservedTraffic, SourceFields, WorkspaceSettings, WorkspaceSettingsUpdate } from "../../protocol.ts";
import { forgetWorkspace, learnWorkspace } from "../../shared/workspace.ts";
import { SettingsView, sourceStatus } from "./SettingsView.tsx";

/** An adapter no server has: the form must come from the list, not from what the page knows. */
const SOURCES: SourceFields[] = [
  {
    source: "graphite",
    label: "Graphite tree",
    fields: [
      { name: "url", label: "Render URL", kind: "url", required: true, secret: false },
      { name: "depth", label: "Tree depth", kind: "number", required: false, secret: false, placeholder: "4" },
      { name: "token", label: "Access token", kind: "text", required: false, secret: true, placeholder: "a token" },
    ],
  },
  { source: "prometheus", label: "Prometheus service graph", fields: [{ name: "url", label: "Query API URL", kind: "url", required: true, secret: false }] },
];

function settings(extra: Partial<WorkspaceSettings> = {}): WorkspaceSettings {
  return {
    id: "shop",
    revision: 4,
    name: "Shop",
    map_repo: "acme/shop-map",
    map_path: "system-map.yml",
    map_ref: "main",
    traffic: { source: "graphite", fields: { url: "http://graphite.lan/render" }, secrets: { token: { set: true } } },
    default_spend_cap_usd: 5,
    updated_by: "ann",
    updated_at: new Date().toISOString(),
    ...extra,
  };
}

type Answer = { status: number; body?: unknown };

/** A server holding `stored`; `put`, `post` and `del` answer the writes. Returns the calls made. */
function serve(stored: WorkspaceSettings, answers: { put?: Answer; post?: Answer; del?: Answer } = {}) {
  const calls: { method: string; url: string; body: unknown }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      calls.push({ method, url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
      const answer = (a: Answer | undefined, fallback: Answer) => {
        const { status, body } = a ?? fallback;
        return new Response(body === undefined ? null : JSON.stringify(body), { status });
      };
      if (url === "/api/traffic/sources") {
        return answer(undefined, { status: 200, body: SOURCES });
      }
      if (url.endsWith("/settings") && method === "GET") {
        return answer(undefined, { status: 200, body: stored });
      }
      if (url.endsWith("/settings") && method === "PUT") {
        return answer(answers.put, { status: 200, body: { ...stored, revision: stored.revision + 1 } });
      }
      if (url === "/api/workspaces" && method === "POST") {
        return answer(answers.post, { status: 201, body: settings({ id: "blog", name: "Blog", revision: 1 }) });
      }
      if (method === "DELETE") {
        return answer(answers.del, { status: 204 });
      }
      return new Response(null, { status: 404 });
    }),
  );
  return calls;
}

function open(ws: string, handlers: Partial<Parameters<typeof SettingsView>[0]> = {}, traffic?: ObservedTraffic) {
  learnWorkspace(ws);
  const props = { onClose: vi.fn(), onChanged: vi.fn(), onNotice: vi.fn(), onSwitch: vi.fn(), ...handlers };
  const view = render(<SettingsView overview={overview({ workspace: ws, ...(traffic ? { traffic } : {}) })} {...props} />);
  return { ...props, view };
}

const save = () => fireEvent.click(screen.getByRole("button", { name: "Save" }));
const sent = (calls: ReturnType<typeof serve>) => calls.find((c) => c.method === "PUT")?.body as WorkspaceSettingsUpdate;

afterEach(() => {
  vi.unstubAllGlobals();
  forgetWorkspace();
});

describe("Configure workspace (issue #189)", () => {
  it("draws the traffic form from the adapter's field list", async () => {
    serve(settings());
    open("shop");
    expect(await screen.findByLabelText("Render URL")).toHaveValue("http://graphite.lan/render");
    expect(screen.getByLabelText("Render URL")).toBeRequired();
    expect(screen.getByLabelText("Tree depth")).toHaveAttribute("type", "number");
    expect(screen.getByLabelText("Tree depth")).toHaveAttribute("placeholder", "4");
    expect(screen.queryByLabelText("Query API URL")).toBeNull();

    // Another adapter, another form.
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "prometheus" } });
    expect(screen.getByLabelText("Query API URL")).toBeInTheDocument();
    expect(screen.queryByLabelText("Render URL")).toBeNull();
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "" } });
    expect(screen.queryByLabelText("Query API URL")).toBeNull();
  });

  it("shows the map's repo and path as fixed and the rest as it is stored", async () => {
    serve(settings());
    open("shop");
    expect(await screen.findByLabelText("Name")).toHaveValue("Shop");
    expect(screen.getByText("acme/shop-map")).toBeInTheDocument();
    expect(screen.getByText("system-map.yml")).toBeInTheDocument();
    expect(screen.queryByDisplayValue("acme/shop-map")).toBeNull();
    expect(screen.getByLabelText("Ref")).toHaveValue("main");
    expect(screen.getByLabelText("Spend cap, USD")).toHaveValue(5);
    expect(screen.getByLabelText("Attempt limit")).toHaveValue(null);
    expect(screen.getByLabelText("Attempt limit")).toHaveAttribute("placeholder", "3");
  });

  it("a secret shows only as set or not set, and is kept unless replaced or cleared", async () => {
    const calls = serve(settings());
    const props = open("shop");
    expect(await screen.findByTestId("settings-secret-state-token")).toHaveTextContent("set");
    expect(screen.queryByLabelText("Access token: new value")).toBeNull();

    // Keep: the default. The update names the secret and carries no value.
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Corner shop" } });
    save();
    await waitFor(() => expect(props.onChanged).toHaveBeenCalledTimes(1));
    expect(sent(calls)).toEqual({
      revision: 4,
      name: "Corner shop",
      map_repo: "acme/shop-map",
      map_path: "system-map.yml",
      map_ref: "main",
      traffic: { source: "graphite", fields: { url: "http://graphite.lan/render" }, secrets: { token: { action: "keep" } } },
      default_spend_cap_usd: 5,
      default_attempt_limit: null,
    });
    expect(calls.find((c) => c.method === "PUT")?.url).toBe("/api/w/shop/settings");
    expect(screen.getByRole("status")).toHaveTextContent("Saved.");

    // Replace: a password box, and its value is sent.
    calls.length = 0;
    const choice = screen.getByLabelText("Access token: keep, replace or clear");
    fireEvent.change(choice, { target: { value: "set" } });
    const box = screen.getByLabelText("Access token: new value");
    expect(box).toHaveAttribute("type", "password");
    fireEvent.change(box, { target: { value: "t0ken" } });
    save();
    await waitFor(() => expect(props.onChanged).toHaveBeenCalledTimes(2));
    expect(sent(calls).traffic?.secrets).toEqual({ token: { action: "set", value: "t0ken" } });

    // Clear.
    calls.length = 0;
    fireEvent.change(screen.getByLabelText("Access token: keep, replace or clear"), { target: { value: "clear" } });
    save();
    await waitFor(() => expect(props.onChanged).toHaveBeenCalledTimes(3));
    expect(sent(calls).traffic?.secrets).toEqual({ token: { action: "clear" } });
  });

  it("a secret that is not set offers Set and no Clear, and a new address warns that a kept one is cleared", async () => {
    serve(settings({ traffic: { source: "graphite", fields: { url: "http://graphite.lan/render" }, secrets: { token: { set: false } } } }));
    open("shop");
    expect(await screen.findByTestId("settings-secret-state-token")).toHaveTextContent("not set");
    const options = within(screen.getByLabelText("Access token: keep, replace or clear")).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual(["Leave unset", "Set"]);
    expect(screen.queryByTestId("settings-secret-cleared-token")).toBeNull();
  });

  it("warns that a kept secret is cleared when the address changes", async () => {
    serve(settings());
    open("shop");
    const url = await screen.findByLabelText("Render URL");
    fireEvent.change(url, { target: { value: "http://graphite.lan/other" } });
    expect(screen.queryByTestId("settings-secret-cleared-token")).toBeNull();
    fireEvent.change(url, { target: { value: "http://elsewhere.example/render" } });
    expect(screen.getByTestId("settings-secret-cleared-token")).toHaveTextContent("will be cleared");
    fireEvent.change(screen.getByLabelText("Access token: keep, replace or clear"), { target: { value: "set" } });
    expect(screen.queryByTestId("settings-secret-cleared-token")).toBeNull();
  });

  it("a stale edit (409) says who changed it and offers their version", async () => {
    const theirs = settings({ revision: 5, name: "Ann's shop", updated_by: "ann" });
    serve(settings(), { put: { status: 409, body: { reason: "changed_meanwhile", message: "someone changed these settings meanwhile; read them again", current: theirs } } });
    const props = open("shop");
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "Bo's shop" } });
    save();
    expect(await screen.findByRole("alert")).toHaveTextContent("ann changed these settings while you were editing. Your edit was not saved.");
    expect(props.onChanged).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Name")).toHaveValue("Bo's shop");
    fireEvent.click(screen.getByRole("button", { name: "Show their version" }));
    expect(screen.getByLabelText("Name")).toHaveValue("Ann's shop");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText(/revision 5/)).toBeInTheDocument();
  });

  it("a refused change (422) shows the server's reason and keeps the edit", async () => {
    serve(settings(), { put: { status: 422, body: { reason: "url_refused", message: "the URL must be http or https" } } });
    const props = open("shop");
    fireEvent.change(await screen.findByLabelText("Render URL"), { target: { value: "ftp://graphite.lan" } });
    save();
    expect(await screen.findByRole("alert")).toHaveTextContent("the URL must be http or https");
    expect(screen.getByLabelText("Render URL")).toHaveValue("ftp://graphite.lan");
    expect(props.onChanged).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Show their version" })).toBeNull();
  });

  it("a refusal with no words of its own (403 from an open server) still says something", async () => {
    serve(settings(), { put: { status: 403, body: { reason: "open_login", message: "this server lets any GitHub account log in, so workspace settings cannot be changed here; configure the GitHub App" } } });
    open("shop");
    await screen.findByLabelText("Name");
    save();
    expect(await screen.findByRole("alert")).toHaveTextContent("cannot be changed here");
  });

  it("a number that does not read is caught before anything is sent", async () => {
    const calls = serve(settings());
    open("shop");
    const cap = await screen.findByLabelText("Spend cap, USD");
    // jsdom lets a number box hold text a browser would refuse.
    Object.defineProperty(cap, "value", { value: "lots", configurable: true });
    fireEvent.change(cap);
    save();
    expect(await screen.findByRole("alert")).toHaveTextContent("The spend cap is a number.");
    expect(calls.some((c) => c.method === "PUT")).toBe(false);
  });

  it("creates a workspace and switches to it; a refusal stays in the form", async () => {
    const calls = serve(settings());
    const props = open("shop");
    fireEvent.click(await screen.findByRole("button", { name: "New workspace…" }));
    const form = screen.getByRole("form", { name: "New workspace" });
    fireEvent.change(within(form).getByLabelText("Id"), { target: { value: "blog" } });
    fireEvent.change(within(form).getByLabelText("Map repo"), { target: { value: "acme/blog-map" } });
    fireEvent.click(within(form).getByRole("button", { name: "Create workspace" }));
    await waitFor(() => expect(props.onSwitch).toHaveBeenCalledWith("blog"));
    expect(calls.find((c) => c.method === "POST")).toEqual({
      method: "POST",
      url: "/api/workspaces",
      body: { id: "blog", name: null, map_repo: "acme/blog-map", map_path: "system-map.yml", map_ref: "main" },
    });
    expect(props.onNotice).toHaveBeenCalledWith("Workspace Blog created.");
  });

  it("a refused create shows why", async () => {
    serve(settings(), { post: { status: 422, body: { reason: "repo_unreadable", message: "you cannot read secret/map on GitHub, so a workspace cannot use it" } } });
    const props = open("shop");
    fireEvent.click(await screen.findByRole("button", { name: "New workspace…" }));
    const form = screen.getByRole("form", { name: "New workspace" });
    fireEvent.change(within(form).getByLabelText("Id"), { target: { value: "hidden" } });
    fireEvent.change(within(form).getByLabelText("Map repo"), { target: { value: "secret/map" } });
    fireEvent.click(within(form).getByRole("button", { name: "Create workspace" }));
    expect(await within(form).findByRole("alert")).toHaveTextContent("you cannot read secret/map on GitHub");
    expect(props.onSwitch).not.toHaveBeenCalled();
  });

  it("deletes a workspace after a second press, by its id, and goes back to default", async () => {
    const calls = serve(settings());
    const props = open("shop");
    fireEvent.click(await screen.findByRole("button", { name: "Delete this workspace…" }));
    expect(calls.some((c) => c.method === "DELETE")).toBe(false);
    expect(screen.getByTestId("settings-delete-confirm")).toHaveTextContent("cannot be used again");
    fireEvent.click(screen.getByRole("button", { name: "Delete shop" }));
    await waitFor(() => expect(props.onSwitch).toHaveBeenCalledWith("default"));
    expect(calls.find((c) => c.method === "DELETE")?.url).toBe("/api/w/shop");
    expect(props.onNotice).toHaveBeenCalledWith(expect.stringContaining("Workspace Shop deleted"));
  });

  it("default cannot be deleted, and its calls go unprefixed", async () => {
    const calls = serve(settings({ id: "default", name: null, traffic: null }));
    open("default");
    await screen.findByLabelText("Name");
    expect(screen.queryByRole("button", { name: "Delete this workspace…" })).toBeNull();
    expect(calls.find((c) => c.url.endsWith("/settings"))?.url).toBe("/api/settings");
    expect(screen.getByLabelText("Source")).toHaveValue("");
  });

  it("says so when the settings cannot be read, and tries again", async () => {
    let fail = true;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (fail) {
          return new Response("the server failed", { status: 500 });
        }
        return new Response(JSON.stringify(url === "/api/traffic/sources" ? SOURCES : settings()), { status: 200 });
      }),
    );
    open("shop");
    expect(await screen.findByRole("alert")).toHaveTextContent("The settings could not be read: the server failed");
    fail = false;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByLabelText("Name")).toHaveValue("Shop");
  });
});

describe("the traffic source's state is this workspace's (issue #281)", () => {
  const NOW = Date.parse("2026-11-18T12:00:00Z");
  const ago = (mins: number) => new Date(NOW - mins * 60_000).toISOString();

  it("each state has its sentence, and no source has none", () => {
    const cases = [
      { t: undefined, says: null },
      { t: { state: "not_configured" as const }, says: null },
      { t: { state: "fresh" as const, observed_at: ago(2), observed_parts: ["web"] }, says: "Working: the last report is from 2m ago." },
      { t: { state: "fresh" as const, observed_at: ago(2), seams: [], undeclared: [] }, says: "Connected, but the source returned no calls between services: the last report is from 2m ago." },
      { t: { state: "stale" as const, observed_at: ago(14) }, says: "No recent report: the last one is from 14m ago." },
      { t: { state: "stale" as const }, says: "No report has arrived yet." },
      { t: { state: "failing" as const, error: "connection refused" }, says: "Failing: connection refused." },
    ];
    for (const c of cases) {
      expect(sourceStatus(c.t, NOW)?.text ?? null).toBe(c.says);
    }
    expect(sourceStatus({ state: "failing" })?.failing).toBe(true);
    console.log(`checked ${cases.length} source states`);
  });

  it("shows the stored source's state, and hides it while the edit differs", async () => {
    serve(settings());
    open("shop", {}, { state: "failing", error: "connection refused", seams: [], undeclared: [] });
    const status = await screen.findByTestId("settings-source-status");
    expect(status).toHaveTextContent("Failing: connection refused.");
    expect(status).toHaveClass("warn");
    fireEvent.change(screen.getByTestId("settings-field-url"), { target: { value: "http://other.lan/render" } });
    expect(screen.queryByTestId("settings-source-status")).toBeNull();
    fireEvent.change(screen.getByTestId("settings-field-url"), { target: { value: "http://graphite.lan/render" } });
    expect(screen.getByTestId("settings-source-status")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "" } });
    expect(screen.queryByTestId("settings-source-status")).toBeNull();
  });

  it("follows the workspace: shop's source works while default has none", async () => {
    serve(settings());
    const { view, ...props } = open("shop", {}, { state: "fresh", observed_at: new Date().toISOString(), seams: [{ seam: "web-server", forward_calls: 2, reverse_calls: 0, errors: 0 }], undeclared: [] });
    expect(await screen.findByTestId("settings-source-status")).toHaveTextContent("Working");
    vi.unstubAllGlobals();
    serve(settings({ id: "default", name: null, traffic: null }));
    learnWorkspace("default");
    view.rerender(<SettingsView overview={overview({ workspace: "default", traffic: { state: "not_configured", seams: [], undeclared: [] } })} {...props} />);
    await waitFor(() => expect(screen.getByLabelText("Source")).toHaveValue(""));
    expect(screen.queryByTestId("settings-source-status")).toBeNull();
  });
});
