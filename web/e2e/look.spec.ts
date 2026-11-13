import { expect, test } from "@playwright/test";

// Issue #81: the restyle loads its fonts from the app's own origin (ADR 0036) and keeps the
// layout inside the viewport at 900 px and below, glows and badges included.

test("fonts load from the app's origin and nowhere else", async ({ page, baseURL }) => {
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

test("must fail: a panel wider than the viewport is caught", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  await page.addStyleTag({ content: ".panel { min-width: 1200px; }" });
  const m = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, right: document.querySelector(".panel")!.getBoundingClientRect().right, inner: window.innerWidth }));
  expect(m.scroll > m.inner || m.right > m.inner).toBe(true);
});

for (const width of [900, 600]) {
  for (const colorScheme of ["dark", "light"] as const) {
    test(`at ${width} px (${colorScheme}) nothing overflows and the panel stays on screen`, async ({ page }) => {
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
