/**
 * Contrast of the colour tokens in `styles.css` against the surfaces they sit on, for the dark
 * scheme (the default, on `:root`) and the light one (issue #81, design §3–4). Small text needs
 * 4.5:1 (WCAG 1.4.3); a role's base colour, used for borders, fills and glows, needs 3:1 on the
 * panel (WCAG 1.4.11). Reports how many pairs, rules and targets it checked.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(import.meta.dirname, "styles.css"), "utf8");

type Tokens = Record<string, string>;

function tokens(block: string): Tokens {
  return Object.fromEntries([...block.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{6})\s*;/g)].map((m) => [m[1] as string, (m[2] as string).toLowerCase()]));
}

/**
 * Dark is `:root { … }`; light is the block inside `@media (prefers-color-scheme: light)` and
 * inherits the rest from dark. `:root[data-theme="light"]` forces light and must repeat that
 * block exactly. Throws when a block is missing, so it can never check one scheme twice.
 */
export function schemes(source: string): { dark: Tokens; light: Tokens } {
  const darkBlock = /^:root\s*\{([^}]*)\}/m.exec(source)?.[1];
  const lightBlock = /prefers-color-scheme:\s*light\)\s*\{\s*:root:not\(\[data-theme="dark"\]\)\s*\{([^}]*)\}/.exec(source)?.[1];
  const forcedBlock = /^:root\[data-theme="light"\]\s*\{([^}]*)\}/m.exec(source)?.[1];
  if (darkBlock === undefined) {
    throw new Error('styles.css has no ":root {" block (the dark scheme)');
  }
  if (lightBlock === undefined) {
    throw new Error('styles.css has no light block in "@media (prefers-color-scheme: light) { :root:not([data-theme="dark"]) {"');
  }
  if (forcedBlock === undefined) {
    throw new Error('styles.css has no ":root[data-theme="light"] {" block');
  }
  const dark = tokens(darkBlock);
  const own = tokens(lightBlock);
  const undefinedInLight = Object.keys(dark).filter((k) => !(k in own));
  if (undefinedInLight.length > 0) {
    throw new Error(`the light block does not define ${undefinedInLight.map((k) => `--${k}`).join(", ")}`);
  }
  const light = { ...dark, ...own };
  const forced = { ...dark, ...tokens(forcedBlock) };
  if (JSON.stringify(forced) !== JSON.stringify(light)) {
    throw new Error(':root[data-theme="light"] does not repeat the prefers-color-scheme: light block');
  }
  return { dark, light };
}

function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map((v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function channels(hex: string): [number, number, number] {
  return [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16)) as [number, number, number];
}

export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** Every surface small text sits on. The part face is a gradient between its two ends. */
const SURFACES = ["bg", "canvas", "panel", "raised", "node-top", "node-bottom"];
/** Accent roles: each has a base (no small text), an -ink (small text) and a -wash (under badges). */
const ROLES = ["accent", "new", "heat", "ok", "warn", "bad"];

/** [text token, background token, where it is used]. */
const PAIRS: [string, string, string][] = [
  ...["ink", "muted", "chip-ink"].flatMap((t) => SURFACES.map((s): [string, string, string] => [t, s, `--${t} text on --${s}`])),
  ...ROLES.flatMap((r) => ["panel", "bg", "raised", "node-top", "node-bottom", `${r}-wash`].map((s): [string, string, string] => [`${r}-ink`, s, `${r} small text on --${s}`])),
  ["on-accent", "accent-strong", "a primary button's label"],
];

/** [lighter end, darker end, where]: text over a gradient is checked at both ends, and the channels must move one way. */
const GRADIENTS: [string, string, string][] = [["node-top", "node-bottom", "the part face"]];

/** Bases for borders, fills and glows: 3:1 on the panel. */
const NON_TEXT = ROLES;

/** Whether every channel of `a` is at least that of `b`, or every one at most: luminance then moves one way along the gradient. */
export function monotone(a: string, b: string): boolean {
  const [x, y] = [channels(a), channels(b)];
  return x.every((v, i) => v >= (y[i] as number)) || x.every((v, i) => v <= (y[i] as number));
}

/** The rule for `selector`, exactly, as its declarations; it must appear once. */
function rule(selector: string): string {
  const at = css.split("\n").filter((l) => l.startsWith(`${selector} {`));
  if (at.length !== 1) {
    throw new Error(`styles.css has ${at.length} rules "${selector} {", want 1`);
  }
  return at[0] as string;
}

/** Whether `declarations` holds `decl` as a declaration of its own: `color:` does not match inside `background-color:`. */
function declares(declarations: string, decl: string): boolean {
  const escaped = decl.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(^|[{;\\s])${escaped}\\s*[;}]`).test(declarations);
}

/** Every rule line that puts text on a role base, or a background on a colour mix the test cannot check. */
export function smallTextScan(source: string): { scanned: number; failures: string[] } {
  const bases = ["accent", "new", "heat", "ok", "warn", "bad", "quiet"];
  const lines = source.split("\n").filter((l) => /^[^\s@/*:][^{]*\{.*\}\s*$/.test(l));
  const failures: string[] = [];
  for (const l of lines) {
    const selector = l.slice(0, l.indexOf(" {"));
    for (const b of bases) {
      if (declares(l, `color: var(--${b})`)) {
        failures.push(`${selector}: text in --${b}, a base colour; use --${b === "quiet" ? "muted" : `${b}-ink`}`);
      }
    }
    if (/(^|[{;\s])background(-color)?:\s*color-mix\(/.test(l)) {
      failures.push(`${selector}: a background mixed from colours; use a -wash or --raised token`);
    }
  }
  return { scanned: lines.length, failures };
}

describe("colour tokens reach their contrast", () => {
  const { light, dark } = schemes(css);
  const both = [
    ["dark", dark],
    ["light", light],
  ] as const;

  it("small text reaches 4.5:1 on every surface, in the dark and the light scheme", () => {
    const failures: string[] = [];
    let checked = 0;
    for (const [name, scheme] of both) {
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
    console.log(`checked ${checked} colour pairs (${PAIRS.length} per scheme, dark and light)`);
    expect(failures).toEqual([]);
    expect(checked).toBe(PAIRS.length * 2);
    expect(PAIRS.length).toBe(55);
    // An idle chip is "clearly above", not just at, the threshold.
    expect(contrast(light["chip-ink"] as string, light["bg"] as string)).toBeGreaterThan(6);
  });

  it("text over a gradient: both ends are surfaces and the channels move one way", () => {
    for (const [name, scheme] of both) {
      for (const [a, b, where] of GRADIENTS) {
        expect(monotone(scheme[a] as string, scheme[b] as string), `${name}: ${where}`).toBe(true);
        expect(SURFACES).toContain(a);
        expect(SURFACES).toContain(b);
      }
    }
    console.log(`checked ${GRADIENTS.length * 2} gradients`);
  });

  it("role bases reach 3:1 on the panel (borders, fills, glows)", () => {
    const failures: string[] = [];
    for (const [name, scheme] of both) {
      for (const r of NON_TEXT) {
        const ratio = contrast(scheme[r] as string, scheme["panel"] as string);
        if (ratio < 3) {
          failures.push(`${name}: --${r} on --panel is ${ratio.toFixed(2)}:1`);
        }
      }
    }
    console.log(`checked ${NON_TEXT.length * 2} non-text colours`);
    expect(failures).toEqual([]);
  });

  it("the rules that carry small text use those tokens", () => {
    const uses: [string, string][] = [
      [".warn", "color: var(--warn-ink)"],
      [".map-node .warn", "color: var(--warn-ink)"],
      [".banner.error", "color: var(--bad-ink)"],
      [".muted", "color: var(--muted)"],
      [".seam-chip", "color: var(--chip-ink)"],
      [".seam-chip", "background: var(--raised)"],
      [".seam-chip.busy", "color: var(--accent-ink)"],
      [".seam-chip.collision", "color: var(--heat-ink)"],
      [".flow-label", "color: var(--chip-ink)"],
      [".flow-label", "background: var(--raised)"],
      ["button.active, .button.primary, button.primary", "color: var(--on-accent)"],
      ["button.active, .button.primary, button.primary", "background: var(--accent-strong)"],
      ...(
        [
          [".badge.designing", "new"],
          [".badge.designed, .badge.building", "accent"],
          [".badge.blocked", "bad"],
          [".badge.in_review", "warn"],
          [".badge.done", "ok"],
          [".badge.working", "ok"],
          [".badge.approved", "ok"],
          [".badge.approval-stale", "warn"],
          [".badge.fits", "ok"],
          [".badge.does-not-fit", "bad"],
        ] as const
      ).flatMap(([sel, role]): [string, string][] => [
        [sel, `color: var(--${role}-ink)`],
        [sel, `background: var(--${role}-wash)`],
      ]),
    ];
    for (const [selector, declaration] of uses) {
      expect(declares(rule(selector), declaration), `${selector} has "${declaration}"`).toBe(true);
    }
    console.log(`checked ${uses.length} rules`);
  });

  it("no rule puts text on a base colour or on a mixed background; body text reaches 4.5:1 on every wash", () => {
    const { scanned, failures } = smallTextScan(css);
    console.log(`scanned ${scanned} rules for text on base colours and mixed backgrounds`);
    expect(failures).toEqual([]);
    for (const [name, scheme] of both) {
      for (const r of ROLES) {
        expect(contrast(scheme["ink"] as string, scheme[`${r}-wash`] as string), `${name}: --ink on --${r}-wash`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("targets a pointer can hit are at least 24px high (WCAG 2.5.8)", () => {
    const targets = [".pin-menu > button", '.pin-menu-list [role="menuitem"]', ".seam-chip", ".lens button", ".tabs button"];
    for (const selector of targets) {
      const px = Number(/min-height:\s*(\d+)px/.exec(rule(selector))?.[1] ?? 0);
      expect(px, selector).toBeGreaterThanOrEqual(24);
    }
    console.log(`checked ${targets.length} target sizes`);
  });

  it("must fail: base tokens are not text tokens, and a broken structure is refused", () => {
    // Fine as borders and fills, not as 12px text.
    expect(contrast(light["accent"] as string, light["bg"] as string)).toBeLessThan(4.5);
    expect(contrast(light["warn"] as string, light["panel"] as string)).toBeLessThan(4.5);
    expect(contrast(dark["quiet"] as string, dark["panel"] as string)).toBeLessThan(4.5);
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    // Without the light block the parser throws instead of checking dark twice.
    const noLight = css.replace(/@media \(prefers-color-scheme: light\)/, "@media (min-width: 1px)");
    expect(() => schemes(noLight)).toThrow(/no light block/);
    // A forced light theme that drifts from the media block is refused.
    const drift = css.replace(/^(:root\[data-theme="light"\] \{[^}]*--ink: )#[0-9a-f]{6}/m, "$1#123456");
    expect(drift).not.toBe(css);
    expect(() => schemes(drift)).toThrow(/does not repeat/);
    // The checker itself: one lowered -ink is reported by the pair loop.
    const lowered: Tokens = { ...light, "warn-ink": "#d97706" };
    const reported = PAIRS.filter(([fg, bg]) => contrast(lowered[fg] as string, lowered[bg] as string) < 4.5).map(([fg]) => fg);
    expect(reported).toContain("warn-ink");
    // A light block that leaves out a colour token is refused, not filled in from dark.
    const missing = css.replace(/(@media \(prefers-color-scheme: light\) \{\n  :root:not\(\[data-theme="dark"\]\) \{[^}]*?)\n    --muted: #[0-9a-f]{6};/, "$1");
    expect(missing).not.toBe(css);
    expect(() => schemes(missing)).toThrow(/does not define --muted/);
    // The scan catches text on a base colour, a mixed background, and `color:` is not read inside `background-color:`.
    expect(smallTextScan(".x { color: var(--accent); }").failures).toHaveLength(1);
    expect(smallTextScan(".x { background: color-mix(in srgb, var(--ok) 8%, var(--panel)); }").failures).toHaveLength(1);
    expect(declares(".x { background-color: var(--muted); }", "color: var(--muted)")).toBe(false);
    // A gradient whose ends cross in one channel is not monotone.
    expect(monotone("#18223a", "#0f2626")).toBe(false);
  });
});
