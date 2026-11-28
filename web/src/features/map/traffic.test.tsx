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
import { PartCard } from "./PartCard.tsx";
import { partTrafficOf, quietState, seamChip, withinText } from "./traffic.tsx";
import { TerritoryMark, TrafficChevrons, TrafficVitals, chevronCount, chipTraffic, chevronsOn, compactRate, health, liveTier, partRates, perMinute, trafficLegend, trafficVitals, unknownSentence } from "./traffic.tsx";

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
    // Light on the pipe it rides (the pipe carries the green, amber or red); amber on territory.
    expect(rule(".traffic")).toMatch(/background: var\(--chevron\)/);
    expect(rule(".traffic.territory")).toMatch(/background: var\(--warn\)/);
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
    expect(rule).toMatch(/stroke: var\(--warn\);.*stroke-dasharray: 1 8;.*stroke-linecap: round/);
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
    { traffic: { state: "not_configured" as const, seams: [], undeclared: [] }, says: "This workspace has no traffic source. Set one in Configure workspace." },
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
    expect(fresh).toContain("Observed calls move as chevrons, more of them for more calls. A hollow seam declares a connection nobody used in the last 1 h. A dotted amber line is traffic the map does not declare, and a dashed card at the right is a service that runs but is not on the map.");
    expect(fresh).toContain("2 services with no namespace and no part to match");
    expect(trafficLegend(withTraffic().traffic!).unmapped).toEqual(["billing", "checkout"]);
  });
});

describe("traffic stands out: pipes, vitals and part rates", () => {
  it("health: under 1 % failed is ok, under 5 % warn, 5 % and up bad; no calls is ok", () => {
    expect([health(0, 0), health(0, 100), health(0.9, 100), health(1, 100), health(4.9, 100), health(5, 100), health(40, 40)]).toEqual(["ok", "ok", "ok", "warn", "warn", "bad", "bad"]);
  });

  it("a live seam's tier is its busier direction's chevron count", () => {
    expect(liveTier({ seam: "s", forward_calls: 0, reverse_calls: 0, errors: 0 }, 3600)).toBe(0);
    expect(liveTier({ seam: "s", forward_calls: 30, reverse_calls: 0, errors: 0 }, 3600)).toBe(1);
    expect(liveTier({ seam: "s", forward_calls: 30, reverse_calls: 3600, errors: 0 }, 3600)).toBe(3);
  });

  it("compact rates read at a glance", () => {
    expect([0, 0.05, 0.46, 7.25, 12.4, 340, 1240, 98_765].map(compactRate)).toEqual(["0", "<0.1", "0.5", "7.3", "12", "340", "1.2k", "98.8k"]);
  });

  it("vitals add every call the report saw: seams, undeclared and ghost edges", () => {
    const t = withTraffic({
      ghost_edges: [{ from: { kind: "ghost", service: { namespace: "shop", name: "checkout" } }, to: { kind: "part", id: "rubato-server" }, calls: 60, errors: 6 }],
      ghosts: [{ service: { namespace: "shop", name: "checkout" }, calls_in: 0, calls_out: 60, errors: 6 }],
    }).traffic as ObservedTraffic;
    const v = trafficVitals(t, 1);
    // 1230 on web-server, 240 undeclared, 60 to a ghost: 1530 calls in 1 h; 42 failed.
    expect(v).toEqual({ perMin: 25.5, calls: 1530, errors: 42, health: "warn", liveSeams: 1, quietSeams: 1, undeclared: 1, ghosts: 1 });
  });

  it("the vitals strip says the numbers in text, with its tone as a class", () => {
    const t = withTraffic().traffic as ObservedTraffic;
    const r = render(<TrafficVitals traffic={t} quietSeams={1} seams={13} now={NOW} />);
    const strip = r.getByTestId("traffic-vitals");
    expect(strip).toHaveTextContent("live calls25/min");
    expect(strip).toHaveTextContent("errors2.4%");
    expect(strip).toHaveTextContent("seams carrying calls1/13");
    expect(strip).toHaveTextContent("quiet seams1");
    expect(strip).toHaveTextContent("not on the map1");
    expect(strip).toHaveTextContent("observed30s ago over 1 h");
    expect(strip.querySelector(".vital.rate")).toHaveClass("warn");
    r.unmount();
  });

  it("part rates: calls a minute in and out of each part, over seams, territory and ghost edges", () => {
    const t = withTraffic().traffic as ObservedTraffic;
    const rates = partRates(t, [{ id: "web-server", from: "web", to: "rubato-server" }]);
    expect(rates.get("web")).toMatchObject({ out: 20, in: 0.5 });
    expect(rates.get("rubato-server")).toMatchObject({ in: 20, out: 0.5, calls: 1230, errors: 36 });
    expect(rates.get("rubato-companion")).toMatchObject({ out: 4, in: 0 });
    expect(rates.get("github")).toMatchObject({ in: 4, out: 0 });
    expect(rates.has("plan-check")).toBe(false);
  });

  it("a part card in the Traffic lens: rates and a glow when observed, then each quiet state (ADR 0083)", () => {
    const g = buildGraph(withTraffic());
    const node = g.nodes.find((n) => n.id === "rubato-server");
    if (!node) throw new Error("no rubato-server");
    const card = () => r.getByTestId("node-rubato-server");
    const r = render(<PartCard node={node} selected={false} traffic={{ in: 20, out: 0.5, calls: 1230, errors: 36 }} />);
    expect(r.getByTestId("part-traffic-rubato-server")).toHaveTextContent("in 20");
    expect(card()).toHaveClass("observed", "health-warn");
    // Seen in a lookback: plain, at full opacity, with the bucket in words.
    r.rerender(<PartCard node={node} selected={false} traffic={{ quiet: "seen", withinSecs: 21_600 }} />);
    expect(card()).toHaveClass("seen");
    expect(card()).not.toHaveClass("unobserved");
    expect(r.getByTestId("part-quiet-rubato-server")).toHaveTextContent("last seen within 6 h");
    expect(r.queryByTestId("part-traffic-rubato-server")).toBeNull();
    // Before any lookback: plain, and nothing is said.
    r.rerender(<PartCard node={node} selected={false} traffic={{ quiet: "unknown" }} />);
    expect(card()).not.toHaveClass("unobserved");
    expect(card()).not.toHaveClass("seen");
    expect(r.queryByTestId("part-quiet-rubato-server")).toBeNull();
    // A lookback answered and did not see it: only now it steps back.
    r.rerender(<PartCard node={node} selected={false} traffic={{ quiet: "not_seen", lookbackSecs: 604_800 }} />);
    expect(card()).toHaveClass("unobserved");
    expect(r.getByTestId("part-quiet-rubato-server")).toHaveTextContent("not seen in 7 d");
    // Nothing can observe it: hatched, never faded.
    r.rerender(<PartCard node={node} selected={false} traffic="not_instrumented" />);
    expect(card()).toHaveClass("blind");
    expect(card()).not.toHaveClass("unobserved");
    expect(r.getByTestId("part-quiet-rubato-server")).toHaveTextContent("not instrumented");
    r.rerender(<PartCard node={node} selected={false} />);
    expect(card()).not.toHaveClass("unobserved");
    expect(card()).not.toHaveClass("blind");
    r.unmount();
  });

  it("quiet states come from the report: last seen, not seen, unknown, not instrumented", () => {
    const t = withTraffic({
      last_seen: [{ ref: { kind: "node", id: "plan-check" }, within_secs: 86_400 }],
      lookback_secs: 604_800,
      not_instrumented: [{ kind: "node", id: "livekit" }, { kind: "seam", id: "web-livekit" }],
    }).traffic as ObservedTraffic;
    expect(quietState("node", "plan-check", t)).toEqual({ quiet: "seen", withinSecs: 86_400 });
    expect(quietState("node", "system-map", t)).toEqual({ quiet: "not_seen", lookbackSecs: 604_800 });
    expect(quietState("node", "livekit", t)).toBe("not_instrumented");
    expect(quietState("seam", "web-livekit", t)).toBe("not_instrumented");
    expect(quietState("node", "system-map", { ...t, lookback_secs: null })).toEqual({ quiet: "unknown" });
    expect([21_600, 86_400, 604_800].map(withinText)).toEqual(["within 6 h", "within a day", "within a week"]);
    // A part that carried calls is its rates, whatever the lookback says.
    const rates = new Map([["plan-check", { in: 1, out: 0, calls: 60, errors: 0 }]]);
    expect(partTrafficOf("plan-check", t, rates)).toEqual({ in: 1, out: 0, calls: 60, errors: 0 });
  });

  it("in the Traffic lens a seam says its rate, that it is quiet, when it was last seen, or that nothing can observe it", () => {
    const t = withTraffic({
      last_seen: [{ ref: { kind: "seam", id: "skill-plan-file" }, within_secs: 21_600 }],
      lookback_secs: 604_800,
      not_instrumented: [{ kind: "seam", id: "web-livekit" }],
    }).traffic as ObservedTraffic;
    const live = { seam: "web-server", forward_calls: 1200, reverse_calls: 30, errors: 36 };
    expect(seamChip({ id: "web-server", traffic: live }, t)).toBe("20.5/min · 3 % errors");
    expect(seamChip({ id: "companion-server", quietFor: 3600 }, t)).toBe("no calls in 1 h");
    expect(seamChip({ id: "skill-plan-file" }, t)).toBe("last seen within 6 h");
    expect(seamChip({ id: "web-livekit" }, t)).toBe("not instrumented");
    expect(seamChip({ id: "server-speech" }, t)).toBe("not seen in 7 d");
    expect(seamChip({ id: "server-speech" }, { ...t, lookback_secs: null })).toBeUndefined();
  });

  it("the headline counts the parts nothing can observe", async () => {
    const o = withTraffic({ not_instrumented: [{ kind: "node", id: "livekit" }, { kind: "node", id: "speech-provider" }, { kind: "seam", id: "web-livekit" }] });
    // The seven numbers are the head of the map versus territory view (issue #357).
    const v = render(<TrafficVitals traffic={o.traffic as ObservedTraffic} quietSeams={0} seams={buildGraph(o).edges.length} />);
    expect(v.getByTestId("traffic-vitals")).toHaveTextContent("not instrumented2");
    v.unmount();
  });

  it("the map opens on Traffic while a report is fresh, and on Structure when it is not", async () => {
    const fresh = render(<MapView graph={buildGraph(withTraffic())} selected={null} onSelect={() => {}} />);
    expect(fresh.getByRole("radio", { name: "Traffic" })).toHaveAttribute("aria-checked", "true");
    // Above the map the observed traffic is one line: the rate, the errors, when, and the way to what the map does not explain.
    const line = await fresh.findByTestId("traffic-line");
    expect(line).toHaveTextContent(/\/min/);
    expect(line).toHaveTextContent(/% errors/);
    expect(line).toHaveTextContent(/observed .* over 1 h/);
    expect(fresh.queryByTestId("traffic-vitals")).toBeNull();
    fresh.unmount();
    const opened: number[] = [];
    const settle = render(<MapView graph={buildGraph(withTraffic())} selected={null} onSelect={() => {}} toSettle={{ count: 3, onOpen: () => opened.push(1) }} />);
    fireEvent.click(await settle.findByTestId("to-settle"));
    expect(settle.getByTestId("to-settle")).toHaveTextContent("3 to settle with the map");
    expect(opened).toEqual([1]);
    settle.unmount();
    const stale = render(<MapView graph={buildGraph(withTraffic({ state: "stale" }))} selected={null} onSelect={() => {}} />);
    expect(stale.getByRole("radio", { name: "Structure" })).toHaveAttribute("aria-checked", "true");
    expect(stale.queryByTestId("traffic-line")).toBeNull();
    stale.unmount();
  });
});
