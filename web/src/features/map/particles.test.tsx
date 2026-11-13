import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Flow } from "../../protocol.ts";
import { arrowsFor } from "./flows.ts";
import { FlowParticles, particleMode, tripSeconds } from "./particles.tsx";

const css = readFileSync(resolve(import.meta.dirname, "../../styles.css"), "utf8");
const flow = (kind: Flow["kind"], from: string, to: string, carries: string): Flow => ({ kind, from, to, carries });
const seam = { id: "web-server", from: "web", to: "rubato-server", flows: [flow("data", "rubato-server", "web", "Overview"), flow("control", "web", "rubato-server", "LaunchRequest")] };
const PATH = "M0,0 C50,0 50,100 100,100";

function draw(arrow: ReturnType<typeof arrowsFor>[number], lens: Parameters<typeof particleMode>[1], opts: { step?: boolean; reduced?: boolean } = {}) {
  const mode = particleMode(arrow, lens, opts.step ?? false);
  const kind = arrow.kind === "structure" ? "data" : arrow.kind;
  return { mode, ...render(<FlowParticles id={arrow.id} path={PATH} kind={kind} reversed={arrow.reversed} mode={mode} reduced={opts.reduced ?? false} length={160} />) };
}

describe("what moves along an arrow (issue #81, design §5)", () => {
  it("proposed stays distinct: a declared control flow carries rings, a proposed flow carries nothing", () => {
    const declared = arrowsFor(seam, "control")[0]!;
    const proposed = arrowsFor(seam, "plan").find((a) => a.kind === "control")!;
    expect(declared.moving).toBe(false);
    expect(declared.dashed).toBe(true);
    expect(proposed.moving).toBe(true);

    const d = draw(declared, "control");
    expect(d.mode).toBe("ambient");
    expect(d.container.querySelectorAll(".particle.control")).toHaveLength(2);
    d.unmount();

    const p = draw(proposed, "plan");
    expect(p.mode).toBe("none");
    expect(p.container.querySelectorAll(".particle, .pulse, .pulse-trail")).toHaveLength(0);

    // Only proposed dashes march; a declared dashed control arrow stands still.
    expect(css).toMatch(/^:root\[data-motion="full"\] \.react-flow__edge\.moving \.react-flow__edge-path \{[^}]*animation:/m);
    expect(css).not.toMatch(/\.react-flow__edge\.flow\.control[^{]*\{[^}]*animation/);
  });

  it("kind by shape: dots for data, rings for control, in the lenses and on the scenario step", () => {
    const data = draw(arrowsFor(seam, "data")[0]!, "data");
    expect(data.container.querySelectorAll(".particle.data")).toHaveLength(2);
    data.unmount();
    // The scenario step: a pulse of the step's kind and a trail of the same shape that is not a pulse.
    const step = draw(arrowsFor(seam, "scenario").find((a) => a.kind === "control")!, "scenario", { step: true });
    expect(step.mode).toBe("step");
    const pulses = step.container.querySelectorAll(".pulse");
    expect(pulses).toHaveLength(1);
    expect(pulses[0]!.classList.contains("control")).toBe(true);
    expect(pulses[0]!.getAttribute("data-testid")).toBe("pulse-web-server#scenario-1");
    const trail = step.container.querySelectorAll(".pulse-trail.control");
    expect(trail).toHaveLength(3);
    expect([...trail].every((t) => !t.classList.contains("pulse") && !t.hasAttribute("data-testid"))).toBe(true);
    step.unmount();
    // The other flows of the scenario lens carry nothing; neither does a faint seam.
    expect(particleMode(arrowsFor(seam, "scenario")[0]!, "scenario", false)).toBe("none");
    expect(particleMode({ ...arrowsFor(seam, "data")[0]!, faint: true }, "data", false)).toBe("none");
    // The shapes themselves: a ring is hollow (canvas inside, a coloured border); a dot is filled.
    const ruleFor = (sel: string) => css.split("\n").find((l) => l.startsWith(`${sel} {`)) ?? "";
    expect(ruleFor(".particle.control")).toMatch(/border: 2px solid var\(--accent\);.*background: var\(--canvas\)/);
    expect(ruleFor(".particle.data")).toMatch(/background: var\(--accent\)/);
    expect(ruleFor(".pulse.control, .pulse-trail.control")).toMatch(/border: 3px solid var\(--accent\);.*background: var\(--canvas\)/);
    expect(ruleFor(".pulse.data, .pulse-trail.data")).toMatch(/background: var\(--accent\)/);
  });

  it("reduced motion: one still marker of the same shape at 30 % of the flow, no trail", () => {
    const reversed = arrowsFor(seam, "data")[0]!;
    expect(reversed.reversed).toBe(true);
    const r = draw(reversed, "data", { reduced: true });
    const marks = r.container.querySelectorAll(".particle");
    expect(marks).toHaveLength(1);
    expect((marks[0] as HTMLElement).style.offsetDistance).toBe("70%");
    expect((marks[0] as HTMLElement).style.animationDuration).toBe("");
    r.unmount();
    const s = draw(arrowsFor(seam, "scenario")[1]!, "scenario", { step: true, reduced: true });
    expect(s.container.querySelectorAll(".pulse-trail")).toHaveLength(0);
    expect((s.container.querySelector(".pulse") as HTMLElement).style.offsetDistance).toBe("30%");
  });

  it("ambient trips take 2 to 6 s, by length", () => {
    expect([tripSeconds(10), tripSeconds(256), tripSeconds(2000)]).toEqual([2, 4, 6]);
  });
});
