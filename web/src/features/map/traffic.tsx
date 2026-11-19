/**
 * Observed traffic on the map (issue #104, ADR 0039, design §5): the Traffic lens's chevrons,
 * the territory mark of calls no seam declares, the chip's rate and the legend's sentences. A
 * chevron is a filled point, so it never reads as a declared flow's dot or ring; how many travel
 * says how many calls, never how fast. Every animation is CSS under `:root[data-motion="full"]`.
 */
import type { CSSProperties } from "react";
import type { Finding, ObservedTraffic, SeamTraffic, TrafficState, UndeclaredEdge } from "../../protocol.ts";
import { relative } from "../../shared/format.ts";
import { windowText } from "../findings/index.ts";
import type { Lens } from "./flows.ts";
import { STILL_AT, tripSeconds } from "./particles.tsx";

/** Calls per minute over a report's window. */
export function perMinute(calls: number, windowSecs: number): number {
  return windowSecs > 0 ? (calls * 60) / windowSecs : 0;
}

/** Chevrons per direction: none, then 1, 2 or 3 for under 1, under 60 and 60+ calls a minute. */
export function chevronCount(perMin: number): 0 | 1 | 2 | 3 {
  if (perMin <= 0) {
    return 0;
  }
  return perMin < 1 ? 1 : perMin < 60 ? 2 : 3;
}

function rateText(perMin: number): string {
  if (perMin < 0.1) {
    return "<0.1/min";
  }
  return `${Math.round(perMin * 10) / 10}/min`;
}

/** What the chip adds in the Traffic lens: the rate both ways, and the error share when any failed. */
export function chipTraffic(t: SeamTraffic, windowSecs: number): string {
  const calls = t.forward_calls + t.reverse_calls;
  const rate = rateText(perMinute(calls, windowSecs));
  return t.errors > 0 && calls > 0 ? `${rate} · ${Math.round((t.errors * 100) / calls)} % errors` : rate;
}

/** Chevrons travel in the Traffic lens only, and only on a fresh report: unknown is never drawn as quiet. */
export function chevronsOn(lens: Lens, state: TrafficState): boolean {
  return lens === "traffic" && state === "fresh";
}

/** How a stream of calls is doing, as a tone: under 1 % failed is ok, under 5 % warn, more bad. */
export type Health = "ok" | "warn" | "bad";

export function health(errors: number, calls: number): Health {
  if (calls <= 0 || errors <= 0) {
    return "ok";
  }
  const share = errors / calls;
  return share < 0.01 ? "ok" : share < 0.05 ? "warn" : "bad";
}

/** How heavy a live seam is drawn: the chevron tier of its busier direction, 0 when no calls. */
export function liveTier(t: SeamTraffic, windowSecs: number): 0 | 1 | 2 | 3 {
  return Math.max(chevronCount(perMinute(t.forward_calls, windowSecs)), chevronCount(perMinute(t.reverse_calls, windowSecs))) as 0 | 1 | 2 | 3;
}

/** A rate for a big number: 0.4, 12, 340, 1.2k. */
export function compactRate(perMin: number): string {
  if (perMin <= 0) {
    return "0";
  }
  if (perMin < 0.1) {
    return "<0.1";
  }
  if (perMin < 10) {
    return String(Math.round(perMin * 10) / 10);
  }
  if (perMin < 1000) {
    return String(Math.round(perMin));
  }
  return `${Math.round(perMin / 100) / 10}k`;
}

/** The headline of a fresh report: every call the source saw, and what the map makes of it. */
export interface Vitals {
  perMin: number;
  calls: number;
  errors: number;
  health: Health;
  /** Declared seams that carried calls. */
  liveSeams: number;
  /** Declared seams with no calls while both their parts show traffic (finding SeamQuiet), as the caller counted them. */
  quietSeams: number;
  /** Calls between parts that no seam joins. */
  undeclared: number;
  /** Services that run but no part names. */
  ghosts: number;
}

export function trafficVitals(t: ObservedTraffic, quietSeams: number): Vitals {
  const window = t.window_secs ?? 3600;
  let calls = 0;
  let errors = 0;
  let liveSeams = 0;
  for (const s of t.seams ?? []) {
    const n = s.forward_calls + s.reverse_calls;
    calls += n;
    errors += s.errors;
    liveSeams += n > 0 ? 1 : 0;
  }
  for (const u of t.undeclared ?? []) {
    calls += u.calls;
    errors += u.errors;
  }
  for (const g of t.ghost_edges ?? []) {
    calls += g.calls;
    errors += g.errors;
  }
  return {
    perMin: perMinute(calls, window),
    calls,
    errors,
    health: health(errors, calls),
    liveSeams,
    quietSeams,
    undeclared: (t.undeclared ?? []).length,
    ghosts: (t.ghosts ?? []).length,
  };
}

/** One part's calls in the Traffic lens: a minute in and out, and the calls and errors behind them. */
export interface PartRate {
  in: number;
  out: number;
  errors: number;
  calls: number;
}

/**
 * A part or seam with no calls in the window (ADR 0083): seen in a longer lookback, not seen in
 * the longest one asked, or unknown because no lookback has answered. Unknown claims nothing.
 */
export type QuietState = { quiet: "seen"; withinSecs: number } | { quiet: "not_seen"; lookbackSecs: number } | { quiet: "unknown" };

/** What a part card shows in the Traffic lens: its rates, a quiet state, or that nothing can observe it. */
export type PartTraffic = PartRate | QuietState | "not_instrumented";

/** Whether `t` is a part's rates rather than a quiet or not-instrumented state. */
export function isRate(t: PartTraffic): t is PartRate {
  return typeof t === "object" && "in" in t;
}

/** The quiet state of a part or seam, or that it is not instrumented (ADR 0083). */
export function quietState(kind: "node" | "seam", id: string, t: ObservedTraffic): QuietState | "not_instrumented" {
  if ((t.not_instrumented ?? []).some((r) => r.kind === kind && r.id === id)) {
    return "not_instrumented";
  }
  const seen = (t.last_seen ?? []).find((l) => l.ref.kind === kind && l.ref.id === id);
  if (seen) {
    return { quiet: "seen", withinSecs: seen.within_secs };
  }
  return t.lookback_secs ? { quiet: "not_seen", lookbackSecs: t.lookback_secs } : { quiet: "unknown" };
}

/** A part's state in the Traffic lens: its rates when it carried calls, else its quiet state. */
export function partTrafficOf(id: string, t: ObservedTraffic, rates: Map<string, PartRate>): PartTraffic {
  const q = quietState("node", id, t);
  if (q === "not_instrumented") {
    return q;
  }
  return rates.get(id) ?? q;
}

/** "within 6 h", "within a day", "within a week": the lookback's bucket in words, never a guessed time. */
export function withinText(secs: number): string {
  if (secs <= 21_600) {
    return `within ${windowText(secs)}`;
  }
  return secs <= 86_400 ? "within a day" : secs <= 604_800 ? "within a week" : `within ${windowText(secs)}`;
}

/** The chip's words for a quiet state; nothing when nothing is known. */
export function quietText(q: QuietState | "not_instrumented"): string | undefined {
  if (q === "not_instrumented") {
    return "not instrumented";
  }
  switch (q.quiet) {
    case "seen":
      return `last seen ${withinText(q.withinSecs)}`;
    case "not_seen":
      return `not seen in ${windowText(q.lookbackSecs)}`;
    case "unknown":
      return undefined;
  }
}

/**
 * What a seam's chip adds in the Traffic lens: its rate when it carried calls, "no calls in 1 h"
 * when it is quiet as a finding, else when it was last seen or that nothing can observe it.
 */
export function seamChip(e: { id: string; quietFor?: number | undefined; traffic?: SeamTraffic | undefined }, t: ObservedTraffic): string | undefined {
  if (e.quietFor !== undefined) {
    return `no calls in ${windowText(e.quietFor)}`;
  }
  if (e.traffic) {
    return chipTraffic(e.traffic, t.window_secs ?? 3600);
  }
  return quietText(quietState("seam", e.id, t));
}

/** The class a quiet state draws with: seen plainly, not seen faded, unknown plain, not instrumented hatched. */
export function quietClass(q: QuietState | "not_instrumented"): string {
  if (q === "not_instrumented") {
    return "blind";
  }
  return q.quiet === "seen" ? "seen" : q.quiet === "not_seen" ? "unobserved" : "";
}

/** Calls a minute into and out of each map part, over declared seams, undeclared edges and ghost edges. */
export function partRates(t: ObservedTraffic, seams: { id: string; from: string; to: string }[]): Map<string, PartRate> {
  const window = t.window_secs ?? 3600;
  const out = new Map<string, PartRate>();
  const add = (from: string | null, to: string | null, calls: number, errors: number) => {
    for (const [id, side] of [
      [from, "out"],
      [to, "in"],
    ] as const) {
      if (id === null || calls <= 0) {
        continue;
      }
      const r = out.get(id) ?? { in: 0, out: 0, errors: 0, calls: 0 };
      r[side] += perMinute(calls, window);
      r.errors += errors;
      r.calls += calls;
      out.set(id, r);
    }
  };
  const byId = new Map(seams.map((s) => [s.id, s]));
  for (const s of t.seams ?? []) {
    const seam = byId.get(s.seam);
    if (!seam) {
      continue;
    }
    const n = s.forward_calls + s.reverse_calls;
    // Errors are not split by direction: each direction takes its share.
    add(seam.from, seam.to, s.forward_calls, n > 0 ? (s.errors * s.forward_calls) / n : 0);
    add(seam.to, seam.from, s.reverse_calls, n > 0 ? (s.errors * s.reverse_calls) / n : 0);
  }
  for (const u of t.undeclared ?? []) {
    add(u.from, u.to, u.calls, u.errors);
  }
  for (const g of t.ghost_edges ?? []) {
    add(g.from.kind === "part" ? g.from.id : null, g.to.kind === "part" ? g.to.id : null, g.calls, g.errors);
  }
  return out;
}

/**
 * The Traffic lens's headline (a fresh report only): the rate, the error share, how many seams
 * carried calls, and what the map does not explain. Numbers are text; colour only repeats them.
 */
export function TrafficVitals({ traffic, quietSeams, seams, now = Date.now() }: { traffic: ObservedTraffic; quietSeams: number; seams: number; now?: number }) {
  const v = trafficVitals(traffic, quietSeams);
  const pct = v.calls > 0 ? (v.errors * 100) / v.calls : 0;
  const offMap = v.undeclared + v.ghosts;
  return (
    <div className="vitals" data-testid="traffic-vitals" role="group" aria-label="Observed traffic">
      <div className={`vital rate ${v.health}`}>
        <span className="vital-label">
          <i className="live-dot" aria-hidden="true" /> live calls
        </span>
        <span className="vital-value">
          {compactRate(v.perMin)}
          <small>/min</small>
        </span>
      </div>
      <div className={`vital ${v.health}`}>
        <span className="vital-label">errors</span>
        <span className="vital-value">
          {pct > 0 && pct < 0.1 ? "<0.1" : Math.round(pct * 10) / 10}
          <small>%</small>
        </span>
      </div>
      <div className="vital seams">
        <span className="vital-label">seams carrying calls</span>
        <span className="vital-value">
          {v.liveSeams}
          <small>/{seams}</small>
        </span>
      </div>
      <div className={v.quietSeams > 0 ? "vital warn" : "vital"}>
        <span className="vital-label">quiet seams</span>
        <span className="vital-value">{v.quietSeams}</span>
      </div>
      <div className={offMap > 0 ? "vital warn" : "vital"}>
        <span className="vital-label">not on the map</span>
        <span className="vital-value">{offMap}</span>
      </div>
      <div className="vital blind" title="Parts with no service on the map: nothing can observe them, so they are never drawn as silent">
        <span className="vital-label">not instrumented</span>
        <span className="vital-value">{(traffic.not_instrumented ?? []).filter((r) => r.kind === "node").length}</span>
      </div>
      <div className="vital when">
        <span className="vital-label">observed</span>
        <span className="vital-value small">
          {traffic.observed_at ? relative(traffic.observed_at, now) : "now"} <small>over {windowText(traffic.window_secs ?? 3600)}</small>
        </span>
      </div>
    </div>
  );
}

/** Why the Traffic lens shows nothing, or null when the report is fresh (design §5.5). */
export function unknownSentence(t: Pick<ObservedTraffic, "state" | "observed_at" | "error">, now = Date.now()): string | null {
  switch (t.state) {
    case "fresh":
      return null;
    case "not_configured":
      return "This workspace has no traffic source. Set one in Configure workspace.";
    case "stale":
      return t.observed_at ? `Traffic unknown: the last report is from ${relative(t.observed_at, now)}.` : "Traffic unknown: no report has arrived yet.";
    case "failing":
      return `Traffic unknown: the source failed: ${t.error ?? "no reason given"}.`;
  }
}

/** The Traffic lens's legend on a fresh report (design §5.6). */
export function trafficLegend(t: ObservedTraffic): { text: string; unmapped: string[]; unmappedCount: number } {
  const window = windowText(t.window_secs ?? 3600);
  return {
    text: `Observed calls move as chevrons, more of them for more calls. A hollow seam declares a connection nobody used in the last ${window}. A dotted amber line is traffic the map does not declare, and a dashed card at the right is a service that runs but is not on the map.`,
    unmapped: t.unmapped_sample ?? [],
    unmappedCount: t.unmapped_services ?? 0,
  };
}

interface ChevronProps {
  id: string;
  /** The edge's SVG path, from the seam's `from` to its `to`. */
  path: string;
  /** Chevrons for calls from the seam's `from` side, and from its `to` side. */
  forward: number;
  reverse: number;
  /** Reduced motion, or past the cap: one still chevron per direction. */
  reduced: boolean;
  length: number;
  /** Calls the map does not declare: amber chevrons on the territory line. */
  territory?: boolean | undefined;
}

export function TrafficChevrons({ id, path, forward, reverse, reduced, length, territory = false }: ChevronProps) {
  const base: CSSProperties = { offsetPath: `path('${path}')` };
  const seconds = tripSeconds(length);
  const tone = territory ? " territory" : "";
  const out = [];
  for (const [dir, n] of [
    ["forward", forward],
    ["reverse", reverse],
  ] as const) {
    if (n <= 0) {
      continue;
    }
    // The rotation is set by the class (offset-rotate); the attribute says it for tests and tools.
    const rotate = dir === "reverse" ? "auto 180deg" : "auto";
    if (reduced) {
      out.push(<div key={dir} className={`traffic ${dir}${tone} still`} aria-hidden="true" data-traffic={id} data-rotate={rotate} style={{ ...base, offsetDistance: `${dir === "reverse" ? 100 - STILL_AT : STILL_AT}%` }} />);
      continue;
    }
    for (let i = 0; i < n; i++) {
      const delay = Math.round((-seconds * i * 1000) / n) / 1000;
      out.push(
        <div key={`${dir}-${i}`} className={`traffic ${dir}${tone}`} aria-hidden="true" data-traffic={id} data-rotate={rotate} style={{ ...base, animationDuration: `${seconds}s`, animationDelay: `${delay}s`, animationDirection: dir === "reverse" ? "reverse" : "normal" }} />,
      );
    }
  }
  return <>{out}</>;
}

interface TerritoryProps {
  edge: UndeclaredEdge;
  windowSecs: number;
  path: string;
  labelX: number;
  labelY: number;
  reduced: boolean;
  /** Past the cap: one still chevron. */
  still: boolean;
  length: number;
}

/**
 * Calls between two parts no seam joins (finding `UndeclaredTraffic`): a label and the calls'
 * chevrons on a static dotted amber line. Not a seam: no button, nothing to select, pin or drop on.
 */
export function TerritoryMark({ edge, windowSecs, path, labelX, labelY, reduced, still, length }: TerritoryProps) {
  const rate = perMinute(edge.calls, windowSecs);
  const id = `territory-${edge.from}-${edge.to}`;
  return (
    <>
      <TrafficChevrons id={id} path={path} forward={chevronCount(rate)} reverse={0} reduced={reduced || still} length={length} territory />
      <div className="seam-label territory-label" data-testid={id} title={`${edge.from} calls ${edge.to}, but no seam on the map joins them`} style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}>
        observed, not on the map · {rateText(rate)}
      </div>
    </>
  );
}

/**
 * The inspector's rows for one seam: calls each way, errors, the window; or that the seam was
 * quiet; or why traffic is unknown. Nothing at all when no source is configured.
 */
export function SeamTrafficRows({ traffic, findings, seam }: { traffic: ObservedTraffic | undefined; findings: Finding[]; seam: { id: string; from: string; to: string } }) {
  if (!traffic || traffic.state === "not_configured") {
    return null;
  }
  const unknown = unknownSentence(traffic);
  const window = traffic.window_secs ?? 3600;
  const t = traffic.seams?.find((s) => s.seam === seam.id);
  const quiet = findings.some((f) => f.kind === "seam_quiet" && f.seam === seam.id);
  const body = unknown ? (
    <p className="muted">{unknown}</p>
  ) : t ? (
    <ul className="plain">
      <li>
        {seam.from} → {seam.to}: {t.forward_calls} calls ({rateText(perMinute(t.forward_calls, window))})
      </li>
      <li>
        {seam.to} → {seam.from}: {t.reverse_calls} calls ({rateText(perMinute(t.reverse_calls, window))})
      </li>
      <li className="muted">
        {t.errors} failed
        {t.forward_calls + t.reverse_calls > 0 && ` (${Math.round((t.errors * 100) / (t.forward_calls + t.reverse_calls))} %)`}, in the last {windowText(window)}
      </li>
    </ul>
  ) : quiet ? (
    <p className="warn">No calls in the last {windowText(window)}, though both its parts show traffic.</p>
  ) : (
    <p className="muted">No calls in the last {windowText(window)}.</p>
  );
  return (
    <>
      <h3>Observed traffic</h3>
      {body}
    </>
  );
}
