/**
 * Every animation and transition in `styles.css` sits behind the motion switch (issue #81,
 * design §5 and §7a): its selector starts with `:root[data-motion="full"]`, which
 * `shared/motion.ts` sets only when the person has not asked for less motion. Reports how many
 * rules it checked.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(import.meta.dirname, "styles.css"), "utf8");
const SWITCH = ':root[data-motion="full"]';

/** Rules that would move something, with their selectors, and those outside the switch. */
export function motionRules(source: string): { checked: number; outside: string[] } {
  // Flatten @media and @supports blocks; drop @keyframes (their steps are not rules).
  const withoutKeyframes = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/@keyframes[^{]*\{(?:[^{}]*\{[^}]*\})*[^}]*\}/g, "");
  const rules = [...withoutKeyframes.matchAll(/([^{};]+)\{([^{}]*)\}/g)].map((m) => ({ selector: (m[1] as string).trim(), body: m[2] as string }));
  let checked = 0;
  const outside: string[] = [];
  for (const { selector, body } of rules) {
    const moves = [...body.matchAll(/(?:^|[;\s])(animation|animation-name|transition|transition-property)\s*:\s*([^;]+)/g)].filter((d) => !/^\s*none\b/.test(d[2] as string));
    if (moves.length === 0) {
      continue;
    }
    checked += 1;
    const all = selector.split(",").map((s) => s.trim());
    if (!all.every((s) => s.startsWith(SWITCH))) {
      outside.push(selector);
    }
  }
  return { checked, outside };
}

describe("the motion switch", () => {
  it("every animation and transition rule is under :root[data-motion=\"full\"]", () => {
    const { checked, outside } = motionRules(css);
    console.log(`checked ${checked} rules that animate or transition`);
    expect(outside).toEqual([]);
    expect(checked).toBeGreaterThan(0);
  });

  it("a belt stops everything under prefers-reduced-motion: reduce", () => {
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\) \{[^}]*animation: none !important;[^}]*transition: none !important;/);
  });

  it("must fail: a rule that animates outside the switch, also inside @media and in a list", () => {
    expect(motionRules(".x { animation: spin 1s linear infinite; }").outside).toEqual([".x"]);
    expect(motionRules("@media (min-width: 1px) { .x { transition: opacity 0.2s; } }").outside).toHaveLength(1);
    expect(motionRules(`${SWITCH} .x, .y { animation: spin 1s; }`).outside).toHaveLength(1);
    expect(motionRules(`${SWITCH} .x { animation: spin 1s; } @keyframes spin { to { transform: rotate(1turn); } }`).outside).toEqual([]);
    expect(motionRules(".x { animation: none; transition: none; }").checked).toBe(0);
  });
});
