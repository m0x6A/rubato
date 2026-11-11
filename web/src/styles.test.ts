/**
 * Contrast of the small-text colour tokens in `styles.css` against the backgrounds they sit on
 * (WCAG 1.4.3, 4.5:1), for the light scheme and the dark one. Reports how many pairs it checked.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(import.meta.dirname, "styles.css"), "utf8");

function tokens(block: string): Record<string, string> {
  return Object.fromEntries([...block.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)].map((m) => [m[1] as string, m[2] as string]));
}

export function schemes(source: string): { light: Record<string, string>; dark: Record<string, string> } {
  const light = tokens(/:root\s*\{([^}]*)\}/.exec(source)?.[1] ?? "");
  const darkBlock = /prefers-color-scheme:\s*dark\)\s*\{\s*:root[^{]*\{([^}]*)\}/.exec(source)?.[1];
  // The dark scheme redefines some tokens and inherits the rest.
  return { light, dark: { ...light, ...tokens(darkBlock ?? "") } };
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = Number.parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** [text token, background token, where it is used]. */
const PAIRS: [string, string, string][] = [
  ["warn-ink", "panel", "repo listing errors and other warnings, 11 to 12px"],
  ["warn-ink", "bg", "warnings on the page background"],
  ["bad-ink", "bg", "a seam chip with a collision"],
  ["bad-ink", "panel", "the error banner"],
  ["chip-ink", "bg", "an idle seam chip"],
  ["accent-ink", "bg", "a seam chip with open plans"],
];

/** The rule for `selector`, exactly, as its declarations. */
function rule(selector: string): string {
  const at = css.split("\n").find((l) => l.startsWith(`${selector} {`));
  if (!at) {
    throw new Error(`styles.css has no rule "${selector} {"`);
  }
  return at;
}

describe("small-text colour tokens reach 4.5:1", () => {
  const { light, dark } = schemes(css);

  it("in the light and the dark scheme", () => {
    const failures: string[] = [];
    let checked = 0;
    for (const [name, scheme] of [["light", light], ["dark", dark]] as const) {
      for (const [fg, bg, where] of PAIRS) {
        const f = scheme[fg];
        const b = scheme[bg];
        if (!f || !b) {
          failures.push(`${name}: --${fg} or --${bg} is not defined`);
          continue;
        }
        checked += 1;
        const ratio = contrast(f, b);
        if (ratio < 4.5) {
          failures.push(`${name}: --${fg} ${f} on --${bg} ${b} is ${ratio.toFixed(2)}:1 (${where})`);
        }
      }
    }
    console.log(`checked ${checked} colour pairs (${PAIRS.length} per scheme, light and dark)`);
    expect(failures).toEqual([]);
    expect(checked).toBe(PAIRS.length * 2);
    // An idle chip is "clearly above", not just at, the threshold.
    expect(contrast(light["chip-ink"] as string, light["bg"] as string)).toBeGreaterThan(6);
  });

  it("the rules that carry small text use those tokens", () => {
    const uses: [string, string][] = [
      [".warn", "--warn-ink"],
      [".map-node .warn", "--warn-ink"],
      [".banner.error", "--bad-ink"],
      [".seam-chip", "--chip-ink"],
      [".seam-chip.busy", "--accent-ink"],
      [".seam-chip.collision", "--bad-ink"],
    ];
    for (const [selector, token] of uses) {
      expect(rule(selector), selector).toContain(`color: var(${token})`);
    }
    console.log(`checked ${uses.length} rules`);
  });

  it("targets a pointer can hit are at least 24px high (WCAG 2.5.8)", () => {
    const targets = [".pin-menu > button", '.pin-menu-list [role="menuitem"]', ".seam-chip"];
    for (const selector of targets) {
      const px = Number(/min-height:\s*(\d+)px/.exec(rule(selector))?.[1] ?? 0);
      expect(px, selector).toBeGreaterThanOrEqual(24);
    }
    console.log(`checked ${targets.length} target sizes`);
  });

  it("must fail: the tokens the small text used before", () => {
    // --warn and --accent in the light scheme are fine as borders and fills, not as 12px text.
    expect(contrast(light["warn"] as string, light["panel"] as string)).toBeLessThan(4.5);
    expect(contrast(light["bad"] as string, light["bg"] as string)).toBeLessThan(4.5);
    expect(contrast(light["accent"] as string, light["bg"] as string)).toBeLessThan(4.5);
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
  });
});
