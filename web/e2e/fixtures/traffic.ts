import type { Page } from "@playwright/test";
import type { ObservedTraffic, Overview } from "../../src/protocol.ts";

/**
 * Observed traffic for the e2e (issue #104): the seeded server has no traffic source, so the real
 * overview is taken and a report added on the way to the page. Fresh: web-server lit both ways,
 * server-github one way, companion-server quiet, and the companion calling GitHub with no seam.
 * Typed as the generated `Overview`, so typecheck catches drift.
 */
export function freshTraffic(o: Overview): Overview {
  const traffic: ObservedTraffic = {
    state: "fresh",
    observed_at: new Date().toISOString(),
    window_secs: 3600,
    seams: [
      { seam: "server-github", forward_calls: 300, reverse_calls: 0, errors: 0 },
      { seam: "web-server", forward_calls: 1200, reverse_calls: 30, errors: 12 },
    ],
    undeclared: [{ from: "rubato-companion", to: "github", calls: 240, errors: 0 }],
    observed_parts: ["github", "rubato-companion", "rubato-server", "web"],
    unmapped_services: 1,
    unmapped_sample: ["checkout"],
  };
  return {
    ...o,
    traffic,
    findings: [...o.findings, { kind: "seam_quiet", seam: "companion-server", window_secs: 3600 }, { kind: "undeclared_traffic", from: "rubato-companion", to: "github", calls: 240 }],
  };
}

/** The same source, gone quiet for 14 minutes: unknown, so nothing is drawn. */
export function staleTraffic(o: Overview): Overview {
  return { ...o, traffic: { state: "stale", observed_at: new Date(Date.now() - 14 * 60_000).toISOString(), window_secs: 3600 } };
}

/** Rewrite every overview the page fetches with `edit`. */
export async function routeOverview(page: Page, edit: (o: Overview) => Overview): Promise<void> {
  await page.route("**/api/overview", async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: edit((await res.json()) as Overview) });
  });
}

/** Directions with calls in `freshTraffic`: web-server both ways, server-github, the territory edge. */
export const FRESH_DIRECTIONS = 4;

/**
 * Issue #140: the fresh report with two services the map does not name, their calls, and one
 * dismissal. Ghosts sit right of the declared map; the Discrepancies list offers the actions.
 */
export function ghostTraffic(o: Overview): Overview {
  const base = freshTraffic(o);
  const checkout = { namespace: "shop", name: "checkout" };
  const payments = { namespace: "shop", name: "payments" };
  return {
    ...base,
    traffic: {
      ...base.traffic,
      state: "fresh",
      ghosts: [
        { service: checkout, calls_in: 0, calls_out: 480, errors: 3 },
        { service: payments, calls_in: 60, calls_out: 0, errors: 0 },
      ],
      ghost_edges: [
        { from: { kind: "ghost", service: checkout }, to: { kind: "part", id: "rubato-server" }, calls: 420, errors: 3 },
        { from: { kind: "ghost", service: checkout }, to: { kind: "ghost", service: payments }, calls: 60, errors: 0 },
      ],
      dismissed: [{ id: 1, target: { kind: "quiet_seam", seam: "server-plan-file" }, author: "m0x6A", at: new Date(Date.now() - 3_600_000).toISOString() }],
    },
    findings: [...base.findings, { kind: "observed_only_part", service: checkout, calls: 480 }, { kind: "observed_only_part", service: payments, calls: 60 }],
  };
}
