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

/** Why the Traffic lens shows nothing, or null when the report is fresh (design §5.5). */
export function unknownSentence(t: Pick<ObservedTraffic, "state" | "observed_at" | "error">, now = Date.now()): string | null {
  switch (t.state) {
    case "fresh":
      return null;
    case "not_configured":
      return "Rubato has no traffic source. Observed traffic is not shown.";
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
    text: `Observed calls move as chevrons, more of them for more calls. A hollow seam declares a connection nobody used in the last ${window}. A dotted amber line is traffic the map does not declare.`,
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
}

export function TrafficChevrons({ id, path, forward, reverse, reduced, length }: ChevronProps) {
  const base: CSSProperties = { offsetPath: `path('${path}')` };
  const seconds = tripSeconds(length);
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
      out.push(<div key={dir} className={`traffic ${dir} still`} aria-hidden="true" data-traffic={id} data-rotate={rotate} style={{ ...base, offsetDistance: `${dir === "reverse" ? 100 - STILL_AT : STILL_AT}%` }} />);
      continue;
    }
    for (let i = 0; i < n; i++) {
      const delay = Math.round((-seconds * i * 1000) / n) / 1000;
      out.push(
        <div key={`${dir}-${i}`} className={`traffic ${dir}`} aria-hidden="true" data-traffic={id} data-rotate={rotate} style={{ ...base, animationDuration: `${seconds}s`, animationDelay: `${delay}s`, animationDirection: dir === "reverse" ? "reverse" : "normal" }} />,
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
      <TrafficChevrons id={id} path={path} forward={chevronCount(rate)} reverse={0} reduced={reduced || still} length={length} />
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
