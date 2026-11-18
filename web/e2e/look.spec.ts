import { expect, test } from "@playwright/test";
import { addCss } from "./fixtures/css.ts";

// Issue #81: the restyle loads its fonts from the app's own origin (ADR 0036) and keeps the
// layout inside the viewport at 900 px and below, glows and badges included.

test("fonts load from the app's origin and nowhere else", { tag: "@look.fonts-from-own-origin" }, async ({ page, baseURL }) => {
  const origin = new URL(baseURL as string).origin;
  const foreign: string[] = [];
  let requests = 0;
  page.on("request", (r) => {
    requests += 1;
    const url = new URL(r.url());
    if (url.protocol.startsWith("http") && url.origin !== origin) {
      foreign.push(r.url());
    }
  });
  await page.goto("/");
  await expect(page.getByTestId("seam-web-server")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  const fonts = await page.evaluate(() => ({
    ui: document.fonts.check("13px 'Space Grotesk Variable'"),
    mono: document.fonts.check("12px 'JetBrains Mono Variable'"),
    loaded: [...document.fonts].filter((f) => f.status === "loaded").map((f) => f.family),
  }));
  console.log(`checked ${requests} requests; ${foreign.length} to another host; fonts loaded: ${[...new Set(fonts.loaded)].join(", ")}`);
  expect(foreign).toEqual([]);
  expect(fonts.ui).toBe(true);
  expect(fonts.mono).toBe(true);
  expect(fonts.loaded).toEqual(expect.arrayContaining(["Space Grotesk Variable", "JetBrains Mono Variable"]));
  // The body and an id use the pairing.
  expect(await page.evaluate(() => getComputedStyle(document.body).fontFamily)).toContain("Space Grotesk Variable");
  expect(await page.getByTestId("node-web").locator(".id").evaluate((e) => getComputedStyle(e).fontFamily)).toContain("JetBrains Mono Variable");
});

test("must fail: a panel wider than the viewport is caught", { tag: ["@look.fits-every-width", "@selftest"] }, async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  await addCss(page, ".panel { min-width: 1200px; }");
  const m = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, right: document.querySelector(".panel")!.getBoundingClientRect().right, inner: window.innerWidth }));
  expect(m.scroll > m.inner || m.right > m.inner).toBe(true);
});

// Issue #193 (ADR 0066): the app runs under its Content-Security-Policy without a violation.
test("the app loads under its Content-Security-Policy with no violation", { tag: "@security.content-security-policy" }, async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __csp: string[] };
    w.__csp = [];
    document.addEventListener("securitypolicyviolation", (e) => w.__csp.push(`${e.violatedDirective} ${e.blockedURI}`));
  });
  const res = await page.goto("/");
  const csp = res?.headers()["content-security-policy"] ?? "";
  expect(csp).toContain("default-src 'self'");
  expect(res?.headers()["x-content-type-options"]).toBe("nosniff");
  await expect(page.getByTestId("seam-web-server")).toBeVisible();
  // A selected part opens the inspector, and the websocket is up: most of what the app draws.
  await page.getByTestId("node-rubato-server").click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  const violations = await page.evaluate(() => (window as unknown as { __csp: string[] }).__csp);
  console.log(`checked the policy "${csp}"; ${violations.length} violations`);
  expect(violations).toEqual([]);
});

test("must fail: an inline style is refused by the policy", { tag: ["@security.content-security-policy", "@selftest"] }, async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  await expect(page.addStyleTag({ content: ".panel { min-width: 1200px; }" })).rejects.toThrow(/Content Security Policy/);
});

// Issue #265: the panel has three sizes while the map sits beside it. The measurements are
// printed; a panel that collapsed to nothing would pass an overflow check alone.
test("the panel widens, fills the window and comes back, and the browser remembers", { tag: "@look.panel-expands" }, async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  const measure = () =>
    page.evaluate(() => {
      const panel = document.querySelector(".panel")!.getBoundingClientRect();
      const map = document.querySelector("section.map")!.getBoundingClientRect();
      return { scroll: document.documentElement.scrollWidth, inner: window.innerWidth, panel: Math.round(panel.width), map: Math.round(map.width) };
    });
  const button = page.getByTestId("panel-size");
  const sizes: Record<string, { panel: number; map: number }> = {};
  for (const size of ["normal", "wide", "full"]) {
    await expect(page.getByTestId("body")).toHaveAttribute("data-panel", size);
    const m = await measure();
    console.log(`panel ${size} at 1440 px: panel ${m.panel}, map ${m.map}, scrollWidth ${m.scroll}`);
    expect(m.scroll).toBeLessThanOrEqual(m.inner);
    sizes[size] = m;
    // The keyboard reaches it: the button is a real one.
    await button.focus();
    await page.keyboard.press("Enter");
  }
  expect(sizes.normal!.panel).toBe(400);
  expect(sizes.wide!.panel).toBeGreaterThan(600);
  expect(sizes.wide!.map).toBeGreaterThan(400);
  expect(sizes.full!.panel).toBeGreaterThanOrEqual(1440 * 0.9);
  expect(sizes.full!.map).toBe(0);
  // Back at normal, the map is there again.
  await expect(page.getByTestId("body")).toHaveAttribute("data-panel", "normal");
  await expect(page.getByTestId("node-web")).toBeVisible();
  // Remembered over a reload.
  await button.click();
  await page.reload();
  await expect(page.getByTestId("body")).toHaveAttribute("data-panel", "wide");
  console.log("checked 3 panel sizes and a reload");
});

test("a stored full size does not hide the map on a narrow window", { tag: "@look.panel-expands" }, async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("rubato.panel", "full"));
  await page.setViewportSize({ width: 600, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  await expect(page.getByTestId("body")).toHaveAttribute("data-panel", "normal");
  await expect(page.getByTestId("panel-size")).toHaveCount(0);
});

for (const width of [900, 600]) {
  for (const colorScheme of ["dark", "light"] as const) {
    test(`at ${width} px (${colorScheme}) nothing overflows and the panel stays on screen`, { tag: "@look.fits-every-width" }, async ({ page }) => {
      await page.emulateMedia({ colorScheme });
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/");
      await expect(page.getByTestId("node-web")).toBeVisible();
      // A selected part and an open inspector: the glows and the panel at their largest.
      await page.getByTestId("node-rubato-server").click();
      await expect(page.getByTestId("inspector")).toBeVisible();
      const m = await page.evaluate(() => {
        const panel = document.querySelector(".panel")!.getBoundingClientRect();
        return { scroll: document.documentElement.scrollWidth, inner: window.innerWidth, panel: { left: panel.left, right: panel.right, top: panel.top } };
      });
      console.log(`at ${width} px: scrollWidth ${m.scroll}, panel ${Math.round(m.panel.left)}–${Math.round(m.panel.right)} from top ${Math.round(m.panel.top)}`);
      expect(m.scroll).toBeLessThanOrEqual(m.inner);
      expect(m.panel.left).toBeGreaterThanOrEqual(0);
      expect(m.panel.right).toBeLessThanOrEqual(m.inner);
      expect(m.panel.top).toBeLessThan(900);
    });
  }
}
