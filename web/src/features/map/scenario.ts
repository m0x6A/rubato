/**
 * Scenarios (ADR 0024): a named journey as ordered steps over the map's declared flows. This
 * resolves each step to the arrow the scenario lens draws for its flow, writes the caption a
 * screen reader hears, and steps the player. Pure, so it is tested without a DOM.
 */
import type { Flow, FlowKind, Scenario } from "../../protocol.ts";

interface SeamLike {
  id: string;
  flows?: Flow[] | undefined;
}

export interface ResolvedStep {
  /** Position in the scenario as declared, 0-based. */
  index: number;
  seam: string;
  from: string;
  to: string;
  kind: FlowKind;
  carries: string;
  says: string | undefined;
  /** The id `arrowsFor(seam, "scenario")` gives this step's flow. */
  arrow: string;
}

export interface ResolvedScenario {
  id: string;
  name: string;
  description: string | undefined;
  steps: ResolvedStep[];
  /** Steps whose flow the map no longer declares exactly once: not drawn. The server refuses
   * such a map, so this is a map changed since the scenario was written in a draft preview. */
  unplaced: number;
}

export function resolveScenario(sc: Scenario, seams: readonly SeamLike[]): ResolvedScenario {
  const byId = new Map(seams.map((s) => [s.id, s]));
  const steps: ResolvedStep[] = [];
  let unplaced = 0;
  sc.steps.forEach((step, index) => {
    const flows = byId.get(step.seam)?.flows ?? [];
    const matches = flows.flatMap((f, i) => (f.kind === step.kind && f.from === step.from && f.to === step.to && (step.carries == null || step.carries === f.carries) ? [{ f, i }] : []));
    const only = matches.length === 1 ? matches[0] : undefined;
    if (!only) {
      unplaced += 1;
      return;
    }
    steps.push({ index, seam: step.seam, from: step.from, to: step.to, kind: step.kind, carries: only.f.carries, says: step.says ?? undefined, arrow: `${step.seam}#scenario-${only.i}` });
  });
  return { id: sc.id, name: sc.name, description: sc.description ?? undefined, steps, unplaced };
}

/** "Step 3 of 8: rubato-server → rubato-companion, control: LaunchCommand — the server sends …" */
export function caption(r: ResolvedScenario, step: number): string {
  const s = r.steps[step];
  if (!s) {
    return `${r.name}: no step to show`;
  }
  return `Step ${step + 1} of ${r.steps.length}: ${s.from} → ${s.to}, ${s.kind}: ${s.carries}${s.says ? ` — ${s.says}` : ""}`;
}

export interface Player {
  /** The step shown, 0-based. */
  step: number;
  playing: boolean;
}

export type PlayerAction = { type: "next" | "prev" | "play" | "pause" | "tick" | "reset"; total: number } | { type: "goto"; total: number; step: number };

export function playerReducer(s: Player, a: PlayerAction): Player {
  const last = Math.max(0, a.total - 1);
  switch (a.type) {
    case "goto":
      return { step: Math.min(last, Math.max(0, a.step)), playing: false };
    case "next":
      return { ...s, step: Math.min(last, s.step + 1) };
    case "prev":
      return { ...s, step: Math.max(0, s.step - 1) };
    case "play":
      // At the end, play starts over.
      return { step: s.step >= last ? 0 : s.step, playing: a.total > 0 };
    case "pause":
      return { ...s, playing: false };
    case "tick":
      return s.step >= last ? { step: last, playing: false } : { ...s, step: s.step + 1 };
    case "reset":
      return { step: 0, playing: false };
  }
}
