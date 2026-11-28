import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import type { DraftOp, GhostPart, ObservedTraffic, Overview, TrafficState } from "../../protocol.ts";
import { Discrepancies } from "./Discrepancies.tsx";
import { NODE_WIDTH, buildGraph } from "./graph.ts";
import { MapView } from "./MapView.tsx";
import { BAND_GAP, discrepancies, ghostBand, ghostId, ghostsToDraw, isGhostId, serviceText, suggestedId } from "./territory.ts";

const shop = (name: string): GhostPart["service"] => ({ namespace: "shop", name });
const checkout: GhostPart = { service: shop("checkout"), calls_in: 0, calls_out: 6, errors: 1 };
const payments: GhostPart = { service: { namespace: "shop-test", name: "payments" }, calls_in: 2, calls_out: 0, errors: 0 };

/** The real overview with a fresh report holding two ghosts, a ghost edge, a quiet seam and a dismissal. */
function observed(traffic: Partial<ObservedTraffic> = {}, ops: DraftOp[] = []): Overview {
  const o = overview();
  return {
    ...o,
    draft: { ...(o.draft ?? {}), ops },
    traffic: {
      state: "fresh",
      observed_at: "2026-11-14T12:00:00Z",
      window_secs: 3600,
      ghosts: [checkout, payments],
      ghost_edges: [{ from: { kind: "ghost", service: shop("checkout") }, to: { kind: "part", id: "rubato-server" }, calls: 5, errors: 1 }],
      dismissed: [{ id: 3, target: { kind: "quiet_seam", seam: "server-github" }, author: "teammate", at: "2026-11-14T11:00:00Z" }],
      ...traffic,
    },
    findings: [...o.findings, { kind: "seam_quiet", seam: "companion-server", window_secs: 3600 }],
  };
}

describe("ghost identity and band (ADR 0059, plan #140 Q5)", () => {
  it("names a ghost by the identity it emits and never like a map id", () => {
    expect(ghostId(shop("checkout"))).toBe("ghost:shop/checkout");
    expect(ghostId({ name: "github" })).toBe("ghost:/github");
    expect(isGhostId("ghost:shop/checkout")).toBe(true);
    expect(isGhostId("rubato-server")).toBe(false);
    expect(serviceText(shop("checkout"))).toBe("shop/checkout");
    expect(serviceText({ name: "github" })).toBe("github");
  });

  it("puts ghosts in one column right of the declared map, sorted, and moves no declared part", () => {
    const declared = new Map([
      ["a", { x: 0, y: 40 }],
      ["b", { x: 500, y: 10 }],
    ]);
    const before = new Map(declared);
    const band = ghostBand(declared, [payments, checkout]);
    expect(declared).toEqual(before);
    const [first, second] = [band.get(ghostId(checkout.service))!, band.get(ghostId(payments.service))!];
    expect(first).toEqual({ x: 500 + NODE_WIDTH + BAND_GAP, y: 10 });
    expect(second.x).toBe(first.x);
    expect(second.y).toBeGreaterThan(first.y);
  });

  it("suggests an id from the name, with the namespace when the name is taken", () => {
    expect(suggestedId(shop("Check Out"), new Set())).toBe("check-out");
    expect(suggestedId(shop("web"), new Set(["web"]))).toBe("shop-web");
  });
});

describe("unknown offers nothing (ADR 0039)", () => {
  for (const state of ["not_configured", "stale", "failing"] as TrafficState[]) {
    it(`${state}: no ghost, no ghost edge, no action`, () => {
      const o = observed({ state });
      expect(ghostsToDraw(o.traffic, new Set(o.map.map.nodes.map((n) => n.id)))).toEqual({ ghosts: [], edges: [] });
      expect(discrepancies(o)).toEqual([]);
      const v = render(<Discrepancies overview={o} onTake={() => {}} onDismiss={() => {}} onUndo={() => {}} />);
      expect(v.container.textContent).toBe("");
      v.unmount();
    });
  }
});

describe("discrepancies and what taking one fills in", () => {
  it("offers a part for a ghost, a seam once its part is drafted, and the seam for a quiet one", () => {
    const rows = discrepancies(observed());
    const ghost = rows.find((r) => r.key.kind === "ghost" && r.key.service.name === "checkout")!;
    expect(ghost.take).toEqual({ kind: "add_node", id: "checkout", nodeKind: "context", service: shop("checkout") });
    const edge = rows.find((r) => r.key.kind === "edge")!;
    expect(edge.take).toEqual({ kind: "blocked", why: "take the part into the draft first" });
    expect(rows.find((r) => r.key.kind === "quiet_seam")!.take).toEqual({ kind: "select_seam", seam: "companion-server" });

    // With the ghost's part in the draft, the edge can follow, between the drafted part and the declared one.
    const drafted: DraftOp = { seq: 1, op_id: "x", author: "m0x6A", created_at: "2026-11-14T12:00:00Z", op: "add_node", node: { id: "shop-checkout", kind: "context", service: shop("checkout") } };
    const after = discrepancies(observed({}, [drafted]));
    expect(after.find((r) => r.key.kind === "edge")!.take).toEqual({ kind: "add_seam", from: "shop-checkout", to: "rubato-server", id: "shop-checkout-rubato-server" });
    expect(after.find((r) => r.key.kind === "ghost" && r.key.service.name === "checkout")!.take).toEqual({ kind: "blocked", why: "in the draft as shop-checkout" });
  });

  it("calls back on a click only, and lists dismissals with who and Undo", () => {
    const onTake = vi.fn();
    const onDismiss = vi.fn();
    const onUndo = vi.fn();
    render(<Discrepancies overview={observed()} onTake={onTake} onDismiss={onDismiss} onUndo={onUndo} now={Date.parse("2026-11-14T12:00:00Z")} />);
    expect(onTake).not.toHaveBeenCalled();
    fireEvent.click(screen.getAllByRole("button", { name: "Add as a part…" })[0]!);
    expect(onTake).toHaveBeenCalledWith({ kind: "add_node", id: "checkout", nodeKind: "context", service: shop("checkout") });
    fireEvent.click(screen.getAllByRole("button", { name: "Dismiss" })[0]!);
    expect(onDismiss).toHaveBeenCalledWith({ kind: "ghost", service: shop("checkout") });
    expect(screen.getByTestId("dismissed").textContent).toContain("server-github quiet · by teammate");
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(onUndo).toHaveBeenCalledWith(3);
    cleanup();
  });

  it("offers no buttons to someone not logged in", () => {
    render(<Discrepancies overview={observed()} onTake={() => {}} onDismiss={() => {}} onUndo={() => {}} canWrite={false} />);
    expect(screen.queryAllByRole("button")).toEqual([]);
    cleanup();
  });
});

describe("ghosts on the map", () => {
  afterEach(() => cleanup());

  it("draws ghosts and their edges in the Traffic lens only, beside parts that keep their place", async () => {
    const o = observed();
    const v = render(<MapView graph={buildGraph(o)} selected={null} onSelect={() => {}} />);
    // A fresh report opens the map on the Traffic lens.
    await v.findByTestId("ghost-shop/checkout", undefined, { timeout: 5000 });
    expect(v.getByRole("radio", { name: "Traffic" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(v.getByRole("radio", { name: "Structure" }));
    const part = await v.findByTestId("node-rubato-server");
    expect(v.queryByTestId("ghost-shop/checkout")).toBeNull();
    const at = (el: HTMLElement) => (el.closest(".react-flow__node") as HTMLElement | null)?.style.transform;
    const before = at(part);
    fireEvent.click(v.getByRole("radio", { name: "Traffic" }));
    const ghost = await v.findByTestId("ghost-shop/checkout");
    expect(ghost.textContent).toContain("not on the map");
    expect(v.getByTestId("ghost-shop-test/payments")).toBeTruthy();
    expect(at(v.getByTestId("node-rubato-server"))).toBe(before);
    // A ghost is not a part: no drop target, nothing to pin.
    expect(ghost.closest("[data-drop]")).toBeNull();
    fireEvent.click(v.getByRole("radio", { name: "Structure" }));
    expect(v.queryByTestId("ghost-shop/checkout")).toBeNull();
  });
});

describe("an empty report is not agreement (issue #285)", () => {
  const fresh = (traffic: Partial<ObservedTraffic>): Overview => ({ ...overview(), findings: [], traffic: { state: "fresh", observed_at: new Date().toISOString(), window_secs: 3600, ...traffic } });
  const show = (o: Overview) => render(<Discrepancies overview={o} onTake={() => {}} onDismiss={() => {}} onUndo={() => {}} />);

  it("nothing observed: says the source returned no calls, and that nothing was compared", () => {
    const v = show(fresh({ seams: [], undeclared: [] }));
    expect(screen.getByTestId("nothing-observed")).toHaveTextContent("The traffic source answered with no calls between services in the last 1 h. Nothing was compared with the map.");
    expect(v.container.textContent).not.toContain("matches the map");
    expect(screen.getByTestId("discrepancies-count")).toHaveTextContent("Map versus territory (nothing observed)");
  });

  it("anything observed and no discrepancy: the observed system matches the map", () => {
    const cases: Partial<ObservedTraffic>[] = [
      { seams: [{ seam: "web-server", forward_calls: 3, reverse_calls: 0, errors: 0 }], observed_parts: ["web", "rubato-server"] },
      { observed_parts: ["web"] },
      { unmapped_services: 2, unmapped_sample: ["a", "b"] },
    ];
    for (const c of cases) {
      const v = show(fresh(c));
      // Must fail on purpose if every empty list reads as "nothing observed".
      expect(v.container.textContent).toContain("The observed system matches the map.");
      expect(screen.queryByTestId("nothing-observed")).toBeNull();
      expect(screen.getByTestId("discrepancies-count")).toHaveTextContent("Map versus territory (0)");
      v.unmount();
    }
    console.log(`checked 1 empty report and ${cases.length} reports with something in them`);
  });
});
