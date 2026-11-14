import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fireEvent, render, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { overview } from "../../../test/fixtures.ts";
import type { ObservedTraffic, Overview } from "../../protocol.ts";
import { LENSES, type Lens } from "./flows.ts";
import { buildGraph } from "./graph.ts";
import { MapView } from "./MapView.tsx";
import { PARTICLE_CAP, particleBudget } from "./particles.tsx";
import { SeamChip } from "./SeamChip.tsx";
import { TerritoryMark, TrafficChevrons, chevronCount, chipTraffic, chevronsOn, perMinute, trafficLegend, unknownSentence } from "./traffic.tsx";

const css = readFileSync(resolve(import.meta.dirname, "../../styles.css"), "utf8");
const PATH = "M0,0 C50,0 50,100 100,100";
const NOW = Date.parse("2026-11-14T12:00:00Z");

/** The real overview with a fresh report: web-server lit both ways, companion-server quiet, the companion calling GitHub directly. */
function withTraffic(traffic: Partial<ObservedTraffic> = {}): Overview {
  const o = overview();
  return {
    ...o,
    traffic: {
      state: "fresh",
      observed_at: new Date(NOW - 30_000).toISOString(),
      window_secs: 3600,
      seams: [{ seam: "web-server", forward_calls: 1200, reverse_calls: 30, errors: 36 }],
      undeclared: [{ from: "rubato-companion", to: "github", calls: 240, errors: 0 }],
      observed_parts: ["web", "rubato-server", "rubato-companion", "github"],
      unmapped_services: 2,
      unmapped_sample: ["billing", "checkout"],
      ...traffic,
    },
    findings: [
      ...o.findings,
      ...(traffic.state && traffic.state !== "fresh"
        ? []
        : [
            { kind: "seam_quiet" as const, seam: "companion-server", window_secs: 3600 },
            { kind: "undeclared_traffic" as const, from: "rubato-companion", to: "github", calls: 240 },
          ]),
    ],
  };
}

describe("volume is how many chevrons travel, not how fast (design §5.3)", () => {
  it("1, 2 or 3 chevrons per direction for under 1, under 60 and 60+ calls per minute", () => {
    const counts = [0, 0.5, 1, 59.9, 60, 6000].map(chevronCount);
    console.log(`chevron counts by calls/min: ${JSON.stringify(counts)}`);
    expect(counts).toEqual([0, 1, 2, 2, 3, 3]);
    expect(perMinute(1200, 3600)).toBe(20);
    expect(perMinute(5, 0)).toBe(0);
  });

  it("the chip says the rate and the error share, as text", () => {
    expect(chipTraffic({ seam: "s", forward_calls: 1200, reverse_calls: 30, errors: 36 }, 3600)).toBe("20.5/min · 3 % errors");
    expect(chipTraffic({ seam: "s", forward_calls: 30, reverse_calls: 0, errors: 0 }, 3600)).toBe("0.5/min");
    expect(chipTraffic({ seam: "s", forward_calls: 1, reverse_calls: 0, errors: 0 }, 86_400)).toBe("<0.1/min");
  });
});

describe("chevrons, by shape (design §5.2)", () => {
  it("point the way the calls go: auto forward, auto 180deg reverse, reversed animation", () => {
    const r = render(<TrafficChevrons id="web-server" path={PATH} forward={3} reverse={1} reduced={false} length={160} />);
    const fwd = r.container.querySelectorAll<HTMLElement>(".traffic.forward");
    const rev = r.container.querySelectorAll<HTMLElement>(".traffic.reverse");
    expect([fwd.length, rev.length]).toEqual([3, 1]);
    expect([...fwd].every((c) => c.dataset["rotate"] === "auto" && c.style.animationDirection === "normal")).toBe(true);
    expect(rev[0]!.dataset["rotate"]).toBe("auto 180deg");
    expect(rev[0]!.style.animationDirection).toBe("reverse");
    // Evenly spread along the path, so three read as more than one.
    expect([...fwd].map((c) => c.style.animationDelay)).toEqual(["0s", "-0.833s", "-1.667s"]);
    // No dot, ring or pulse: those are declared flows.
    expect(r.container.querySelectorAll(".particle, .pulse, .pulse-trail")).toHaveLength(0);
  });

  it("reduced motion: one still chevron per direction, 30 % from its own start", () => {
    const r = render(<TrafficChevrons id="web-server" path={PATH} forward={3} reverse={2} reduced={true} length={160} />);
    const all = r.container.querySelectorAll<HTMLElement>(".traffic");
    expect(all).toHaveLength(2);
    expect([...all].map((c) => c.style.offsetDistance)).toEqual(["30%", "70%"]);
    expect([...all].every((c) => c.classList.contains("still") && c.style.animationDuration === "")).toBe(true);
  });

  it("the chevron's rule moves only under the motion switch, and its shape is a clipped point", () => {
    const rule = (sel: string) => css.split("\n").find((l) => l.startsWith(`${sel} {`)) ?? "";
    expect(rule(".traffic")).toMatch(/clip-path: polygon\(0 0, 55% 50%, 0 100%, 40% 100%, 95% 50%, 40% 0\)/);
    expect(rule(".traffic")).toMatch(/background: var\(--ok\)/);
    expect(rule(".traffic")).not.toMatch(/animation/);
    expect(css).toMatch(/^:root\[data-motion="full"\] \.traffic \{[^}]*animation-name: travel/m);
    expect(css).toMatch(/^\.traffic\.forward \{ offset-rotate: auto; \}$/m);
    expect(css).toMatch(/^\.traffic\.reverse \{ offset-rotate: auto 180deg; \}$/m);
    // Every line that animates a chevron sits under the switch; must fail on one that does not.
    const unswitched = (source: string) => source.split("\n").filter((l) => /\.traffic\b/.test(l) && /animation/.test(l) && !l.startsWith(':root[data-motion="full"]'));
    expect(unswitched(css)).toEqual([]);
    expect(unswitched(".traffic { animation-name: travel; }")).toHaveLength(1);
  });

  it("chevrons only in the Traffic lens, and only while the report is fresh", () => {
    const lenses: Lens[] = ["structure", "data", "control", "plan", "scenario", "traffic"];
    const on = lenses.filter((l) => chevronsOn(l, "fresh"));
    expect(on).toEqual(["traffic"]);
    expect((["not_configured", "stale", "failing"] as const).filter((s) => chevronsOn("traffic", s))).toEqual([]);
    expect(LENSES.map((l) => l.label)).toEqual(["Structure", "Data flow", "Control flow", "Traffic"]);
  });
});

describe("the particle cap counts chevrons (design §5.3)", () => {
  it("count 2 keeps the results it had", () => {
    expect(particleBudget(Array.from({ length: 60 }, (_, i) => ({ id: `f${i}`, near: false }))).size).toBe(60);
    expect(particleBudget(Array.from({ length: 200 }, (_, i) => ({ id: `f${i}`, near: false, count: 2 }))).size).toBe(60);
  });

  it("30 traffic seams at 6 chevrons (180) are cut to 120, near seams first", () => {
    const seams = Array.from({ length: 30 }, (_, i) => ({ id: `s${i}`, near: i >= 25, count: 6 }));
    const moving = particleBudget(seams);
    const used = seams.filter((s) => moving.has(s.id)).reduce((n, s) => n + s.count, 0);
    console.log(`traffic cap: ${moving.size} of ${seams.length} seams move, ${used} chevrons`);
    expect(used).toBe(PARTICLE_CAP);
    for (let i = 25; i < 30; i++) {
      expect(moving.has(`s${i}`)).toBe(true);
    }
    expect(moving.has("s29") && !moving.has("s20")).toBe(true);
  });
});

describe("findings on conduits (design §5.4)", () => {
  it("buildGraph puts calls on their seam, marks the quiet seam and keeps the territory edge", () => {
    const g = buildGraph(withTraffic());
    const ws = g.edges.find((e) => e.id === "web-server")!;
    const quiet = g.edges.find((e) => e.id === "companion-server")!;
    expect(ws.traffic?.forward_calls).toBe(1200);
    expect(ws.quietFor).toBeUndefined();
    expect(quiet.quietFor).toBe(3600);
    // Amber quiet, not magenta collision: a quiet seam is not a seam finding of a plan.
    expect(quiet.findings).toEqual([]);
    expect(quiet.collision).toBe(false);
    expect(g.territory).toEqual([{ from: "rubato-companion", to: "github", calls: 240, errors: 0 }]);
  });

  it("a quiet seam's chip is amber, says so in the Traffic lens and keeps its ⚠ in every lens", () => {
    const quiet = buildGraph(withTraffic()).edges.find((e) => e.id === "companion-server")!;
    const inTraffic = render(<SeamChip seam={quiet} selected={false} onSelect={() => {}} traffic="no calls in 1 h" />);
    const chip = inTraffic.getByTestId("seam-companion-server");
    expect(chip.classList.contains("quiet")).toBe(true);
    expect(chip.classList.contains("collision")).toBe(false);
    expect(chip).toHaveTextContent("companion-server · no calls in 1 h");
    expect(chip).toHaveTextContent("⚠");
    expect(chip).toHaveTextContent("no calls observed on this seam in the last 1 h, though both its parts show traffic");
    inTraffic.unmount();
    const elsewhere = render(<SeamChip seam={quiet} selected={false} onSelect={() => {}} />);
    const c = elsewhere.getByTestId("seam-companion-server");
    expect(c).not.toHaveTextContent("no calls in 1 h ·");
    expect(c).toHaveTextContent("⚠");
    expect(within(c).getByText(/no calls observed on this seam/)).toHaveClass("sr-only");
  });

  it("undeclared traffic is a territory mark: its own test id, a label, chevrons, no button", () => {
    const r = render(<TerritoryMark edge={{ from: "rubato-companion", to: "github", calls: 240, errors: 0 }} windowSecs={3600} path={PATH} labelX={50} labelY={50} reduced={false} still={false} length={160} />);
    const mark = r.getByTestId("territory-rubato-companion-github");
    expect(mark).toHaveTextContent("observed, not on the map · 4/min");
    expect(r.container.querySelectorAll(".traffic.forward")).toHaveLength(2);
    expect(r.container.querySelectorAll(".traffic.reverse")).toHaveLength(0);
    expect(r.container.querySelector("button")).toBeNull();
    expect(r.container.querySelector(".moving")).toBeNull();
    // Static dotted amber, round caps; never the moving class's march.
    const rule = css.split("\n").find((l) => l.startsWith(".react-flow__edge.territory .react-flow__edge-path {")) ?? "";
    expect(rule).toMatch(/stroke: var\(--warn\);.*stroke-dasharray: 1 5;.*stroke-linecap: round/);
    expect(css).not.toMatch(/\.territory[^{]*\{[^}]*animation/);
  });

  it("the hollow quiet conduit: rim in amber, core the canvas", () => {
    expect(css).toMatch(/^\.react-flow__edge\.quiet \.conduit \{[^}]*stroke: var\(--warn\)/m);
    expect(css).toMatch(/^\.react-flow__edge\.quiet \.react-flow__edge-path \{[^}]*stroke: var\(--canvas\)/m);
    expect(css).not.toMatch(/\.quiet[^{]*\{[^}]*animation/);
  });
});

describe("unknown, not quiet (design §5.5)", () => {
  const states = [
    { traffic: { state: "not_configured" as const, seams: [], undeclared: [] }, says: "Rubato has no traffic source. Observed traffic is not shown." },
    { traffic: { state: "stale" as const, observed_at: new Date(NOW - 14 * 60_000).toISOString(), seams: [], undeclared: [] }, says: "Traffic unknown: the last report is from 14m ago." },
    { traffic: { state: "failing" as const, error: "dev traffic file: No such file", seams: [], undeclared: [] }, says: "Traffic unknown: the source failed: dev traffic file: No such file." },
  ];

  it("each non-fresh state has its sentence, and fresh has none", () => {
    for (const s of states) {
      expect(unknownSentence(s.traffic, NOW)).toBe(s.says);
    }
    expect(unknownSentence({ state: "stale" }, NOW)).toBe("Traffic unknown: no report has arrived yet.");
    expect(unknownSentence(withTraffic().traffic!, NOW)).toBeNull();
  });

  it("a non-fresh overview draws no traffic, quiet seam or territory edge, even if a stray one is sent", () => {
    for (const s of states) {
      const o = withTraffic({ ...s.traffic, seams: [{ seam: "web-server", forward_calls: 9, reverse_calls: 0, errors: 0 }], undeclared: [{ from: "web", to: "github", calls: 9, errors: 0 }] });
      o.findings.push({ kind: "seam_quiet", seam: "companion-server", window_secs: 3600 });
      const g = buildGraph(o);
      expect(g.edges.some((e) => e.traffic || e.quietFor), s.traffic.state).toBe(false);
      expect(g.territory, s.traffic.state).toEqual([]);
    }
  });

  it("the legend in the Traffic lens says why, or what the marks mean", async () => {
    const legendFor = async (o: Overview) => {
      const v = render(<MapView graph={buildGraph(o)} selected={null} onSelect={() => {}} />);
      await v.findByTestId("node-rubato-server", undefined, { timeout: 5000 });
      fireEvent.click(v.getByRole("radio", { name: "Traffic" }));
      const text = v.getByTestId("legend").textContent ?? "";
      v.unmount();
      return text;
    };
    expect(await legendFor(withTraffic({ state: "failing", error: "refused", seams: [], undeclared: [] }))).toContain("Traffic unknown: the source failed: refused.");
    const fresh = await legendFor(withTraffic());
    expect(fresh).toContain("Observed calls move as chevrons, more of them for more calls. A hollow seam declares a connection nobody used in the last 1 h. A dotted amber line is traffic the map does not declare.");
    expect(fresh).toContain("2 services are not on the map");
    expect(trafficLegend(withTraffic().traffic!).unmapped).toEqual(["billing", "checkout"]);
  });
});
