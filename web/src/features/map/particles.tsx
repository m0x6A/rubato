/**
 * What moves along an arrow (issue #81, design §5). Two vocabularies, never on one edge:
 * particles travel on declared flows, dashes march on proposed ones (`.moving`, CSS only).
 * Data is a filled dot and control a ring, so the kind reads without colour. Every animation is
 * CSS, named only under `:root[data-motion="full"]`; with reduced motion one still marker of the
 * same shape sits just before the flow's label (after it, for a flow against the path), so the
 * label never covers it on a short curve; with no label known, at 30 % of the path.
 */
import type { CSSProperties } from "react";
import type { Arrow, Lens } from "./flows.ts";

export type ParticleMode = "ambient" | "step" | "none";

/** Ambient particles in the data and control lenses on declared flows; the step's pulse in the scenario lens; nothing on a proposed flow. */
export function particleMode(arrow: Arrow, lens: Lens, step: boolean): ParticleMode {
  if (arrow.kind === "structure" || arrow.moving) {
    return "none";
  }
  if (step) {
    return "step";
  }
  if ((lens === "data" || lens === "control") && !arrow.faint) {
    return "ambient";
  }
  return "none";
}

/** Pixels per second for an ambient particle: it shows direction, not volume. */
export const PARTICLE_SPEED = 64;
/** One trip of the scenario pulse. */
export const PULSE_MS = 1200;
const TRAIL = [60, 120, 180];
/** Where the still marker sits, from the flow's own start, when its label's place is not known. */
export const STILL_AT = 30;
/** Room between the still marker's centre and its label's edge, in pixels. */
export const STILL_GAP = 12;

/** Where the flow's label sits: its place along the path (0–1) and half its width in pixels. */
export interface LabelPlace {
  at: number;
  half: number;
}

/**
 * The still marker's `offset-distance`: on the flow's own start side of its label, clear of it
 * on any length of curve, and never off the path. Without a label, at `STILL_AT` %.
 */
export function stillDistance(reversed: boolean, label?: LabelPlace): string {
  if (!label) {
    return `${reversed ? 100 - STILL_AT : STILL_AT}%`;
  }
  const at = Math.round(label.at * 1000) / 10;
  const clear = Math.ceil(label.half + STILL_GAP);
  return reversed ? `min(96%, calc(${at}% + ${clear}px))` : `max(4%, calc(${at}% - ${clear}px))`;
}

/** How long one ambient trip takes along a path of this length, clamped to 2–6 s. */
export function tripSeconds(length: number): number {
  return Math.min(6, Math.max(2, length / PARTICLE_SPEED));
}

interface Props {
  /** The arrow's id: the pulse's test id is `pulse-<id>`. */
  id: string;
  /** The edge's SVG path, in flow coordinates, from the seam's source to its target. */
  path: string;
  kind: "data" | "control";
  /** The flow runs against the path: travel from its end. */
  reversed: boolean;
  mode: ParticleMode;
  reduced: boolean;
  /** About how long the path is, for the ambient trip time. */
  length: number;
  /** Where the flow's label sits, so the still marker stays clear of it. */
  label?: LabelPlace;
}

export function FlowParticles({ id, path, kind, reversed, mode, reduced, length, label }: Props) {
  if (mode === "none") {
    return null;
  }
  const base: CSSProperties = { offsetPath: `path('${path}')` };
  const still: CSSProperties = { ...base, offsetDistance: stillDistance(reversed, label) };
  const direction = reversed ? "reverse" : "normal";
  if (mode === "step") {
    if (reduced) {
      return <div className={`pulse ${kind} still`} data-testid={`pulse-${id}`} style={still} />;
    }
    const run: CSSProperties = { ...base, animationDuration: `${PULSE_MS}ms`, animationDirection: direction };
    return (
      <>
        {TRAIL.map((lag, i) => (
          <div key={lag} className={`pulse-trail ${kind} t${i + 1}`} aria-hidden="true" style={{ ...run, animationDelay: `${lag}ms` }} />
        ))}
        <div className={`pulse ${kind}`} data-testid={`pulse-${id}`} style={run} />
      </>
    );
  }
  if (reduced) {
    return <div className={`particle ${kind} still`} aria-hidden="true" data-flow={id} style={still} />;
  }
  const seconds = tripSeconds(length);
  return (
    <>
      {[0, -seconds / 2].map((delay) => (
        <div key={delay} className={`particle ${kind}`} aria-hidden="true" data-flow={id} style={{ ...base, animationDuration: `${seconds}s`, animationDelay: `${delay}s`, animationDirection: direction }} />
      ))}
    </>
  );
}

/** At most this many moving particles on the map at once (design §5): dots, rings and chevrons. */
export const PARTICLE_CAP = 120;
/** A declared flow runs two particles; a traffic seam passes its chevron count (up to 6, #104). */
const PER_FLOW = 2;

/**
 * Which ambient items keep moving when their particles are more than the cap allows: first the
 * items next to the selection or a lit part, then the rest in map order, while their counts fit.
 * The others show the still marker. Returns the ids of the items that move.
 */
export function particleBudget(ambient: { id: string; near: boolean; count?: number }[], cap = PARTICLE_CAP): Set<string> {
  const count = (a: { count?: number }) => a.count ?? PER_FLOW;
  if (ambient.reduce((n, a) => n + count(a), 0) <= cap) {
    return new Set(ambient.map((a) => a.id));
  }
  const ordered = [...ambient.filter((a) => a.near), ...ambient.filter((a) => !a.near)];
  const moving = new Set<string>();
  let used = 0;
  for (const a of ordered) {
    if (used + count(a) > cap) {
      break;
    }
    used += count(a);
    moving.add(a.id);
  }
  return moving;
}
