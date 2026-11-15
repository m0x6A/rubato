import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import type { Overview, ServiceId } from "../../protocol.ts";
import { DraftPanel } from "./DraftPanel.tsx";

const shop = (name: string): ServiceId => ({ namespace: "shop", name });

describe("the prefilled draft form (issue #140)", () => {
  let calls: { method: string; url: string; body: Record<string, unknown> | null }[];
  beforeEach(() => {
    calls = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        calls.push({ method: init?.method ?? "GET", url, body: init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : null });
        return new Response(null, { status: 201 });
      }),
    );
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    cleanup();
  });

  it("fills in Add part from the ghost, sends nothing until submitted, then sends its service", async () => {
    render(<DraftPanel overview={overview()} selection={null} onChanged={() => {}} onNotice={() => {}} prefill={{ nonce: 1, op: "add_node", id: "checkout", kind: "context", service: shop("checkout") }} />);
    const form = screen.getByRole("form", { name: "Add part" });
    expect((form.querySelector("input[name=id]") as HTMLInputElement).value).toBe("checkout");
    expect((form.querySelector("input[name=service_namespace]") as HTMLInputElement).value).toBe("shop");
    expect((form.querySelector("input[name=service_name]") as HTMLInputElement).value).toBe("checkout");
    expect(calls).toEqual([]);
    fireEvent.submit(form);
    await waitFor(() => expect(calls.length).toBe(1));
    expect(calls[0]!.body).toMatchObject({ op: "add_node", node: { id: "checkout", kind: "context", service: { namespace: "shop", name: "checkout" } } });
  });

  it("keeps a part's service when the part is changed", async () => {
    const o = overview();
    const node = o.map.map.nodes[0]!;
    const withService: Overview = { ...o, map: { ...o.map, map: { ...o.map.map, nodes: [{ ...node, service: { namespace: "rubato", name: node.id } }, ...o.map.map.nodes.slice(1)] } } };
    render(<DraftPanel overview={withService} selection={{ kind: "node", id: node.id }} onChanged={() => {}} onNotice={() => {}} />);
    fireEvent.submit(screen.getByRole("form", { name: `Change part ${node.id}` }));
    await waitFor(() => expect(calls.length).toBe(1));
    expect(calls[0]!.body).toMatchObject({ op: "update_node", node: { id: node.id, service: { namespace: "rubato", name: node.id } } });
  });

  it("fills in Add seam from an observed edge", () => {
    render(<DraftPanel overview={overview()} selection={null} onChanged={() => {}} onNotice={() => {}} prefill={{ nonce: 2, op: "add_seam", from: "web", to: "github", id: "web-github" }} />);
    const form = screen.getByRole("form", { name: "Add seam" });
    expect((form.querySelector("select[name=from]") as HTMLSelectElement).value).toBe("web");
    expect((form.querySelector("select[name=to]") as HTMLSelectElement).value).toBe("github");
    expect((form.querySelector("input[name=id]") as HTMLInputElement).value).toBe("web-github");
    expect(calls).toEqual([]);
  });
});
