import { type Page, expect, test } from "@playwright/test";

// Issue #81, design §7a: with reduced motion nothing moves. Proven twice: once with the switch
// alone (the media query says no-preference, so the CSS belt is not what stops anything, and
// `matchMedia` is made to answer "reduce"), once with the real media query. The must-fail run
// is the same page with full motion.

const REDUCE_QUERY = "(prefers-reduced-motion: reduce)";

/** Make `matchMedia` answer "reduce" without touching the CSS media query. */
async function forceReducedSwitch(page: Page): Promise<void> {
  await page.addInitScript((q) => {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (query: string) => {
      if (query !== q) {
        return real(query);
      }
      const none = () => {};
      return { matches: true, media: query, onchange: null, addEventListener: none, removeEventListener: none, addListener: none, removeListener: none, dispatchEvent: () => false } as unknown as MediaQueryList;
    };
  }, REDUCE_QUERY);
}

/** Do `act` in the page, wait 50 ms (transitions are 120–480 ms), and count what animates. */
async function animationsAfter(page: Page, act: string): Promise<{ running: number; smil: number }> {
  return page.evaluate(async (selector) => {
    (document.querySelector(selector) as HTMLElement | null)?.click();
    await new Promise((r) => setTimeout(r, 50));
    return { running: document.getAnimations().length, smil: document.querySelectorAll("animateMotion, animate, animateTransform").length };
  }, act);
}

async function setUp(page: Page): Promise<void> {
  const pin = await page.request.post("/api/pins", { data: { issue: { repo: "m0x6A/rubato", number: 901 }, target: { kind: "node", id: "rubato-server" } } });
  expect([200, 201, 409]).toContain(pin.status());
  await page.goto("/");
  await expect(page.getByTestId("seam-web-server")).toBeVisible();
  await expect(page.getByTestId("issue-row-901")).toContainText("Designed", { timeout: 20_000 });
}

/** Pixels that may differ between two frames of a still canvas: none. A pulse or particle moving for 500 ms changes thousands. */
const STILL_NOISE_PX = 0;

/** How many pixels differ, decoded in a browser canvas (no PNG library in this repo). */
async function differingPixels(page: Page, a: Buffer, b: Buffer): Promise<number> {
  const scratch = await page.context().newPage();
  const n = await scratch.evaluate(async ([a, b]) => {
    const load = (src: string) =>
      new Promise<HTMLImageElement>((ok) => {
        const i = new Image();
        i.onload = () => ok(i);
        i.src = src;
      });
    const [ia, ib] = await Promise.all([load(a), load(b)]);
    const px = (img: HTMLImageElement) => {
      const c = document.createElement("canvas");
      c.width = img.width;
      c.height = img.height;
      const x = c.getContext("2d")!;
      x.drawImage(img, 0, 0);
      return x.getImageData(0, 0, c.width, c.height).data;
    };
    const [da, db] = [px(ia), px(ib)];
    let n = 0;
    for (let i = 0; i < da.length; i += 4) {
      if (da[i] !== db[i] || da[i + 1] !== db[i + 1] || da[i + 2] !== db[i + 2]) {
        n += 1;
      }
    }
    return n;
  }, [`data:image/png;base64,${a.toString("base64")}`, `data:image/png;base64,${b.toString("base64")}`] as const);
  await scratch.close();
  return n;
}

/** Two screenshots of the map canvas 500 ms apart, Play paused: how many pixels changed. */
async function canvasChange(page: Page): Promise<number> {
  const canvas = page.locator(".map-canvas");
  // Flow labels and seam chips are hidden for the comparison: React Flow re-places them by a
  // sub-pixel when it re-measures, which is not motion (they carry no animation; the counts above
  // cover them). Then the canvas settles: hiding them and the lens switch both repaint, and
  // Chromium draws coarse tiles first. Motion would go on regardless.
  const style = await page.addStyleTag({ content: ".seam-label { visibility: hidden !important; }" });
  await page.waitForTimeout(500);
  // The first capture after a repaint makes Chromium re-rasterise once more; it is not compared.
  await canvas.screenshot({ animations: "allow" });
  const a = await canvas.screenshot({ animations: "allow" });
  await page.waitForTimeout(500);
  const b = await canvas.screenshot({ animations: "allow" });
  await style.evaluate((e) => (e as Element).remove());
  return differingPixels(page, a, b);
}

const runs = [
  { name: "the switch alone (media: no-preference, matchMedia: reduce)", media: "no-preference", force: true },
  { name: "the media query (reduce)", media: "reduce", force: false },
] as const;

for (const run of runs) {
  test(`reduced motion, ${run.name}: nothing animates`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: run.media });
    if (run.force) {
      await forceReducedSwitch(page);
    }
    await setUp(page);
    expect(await page.evaluate(() => document.documentElement.dataset["motion"])).toBe("reduced");

    const counts: Record<string, { running: number; smil: number }> = {};
    // A part selected: the glow and the inspector.
    counts["select a part"] = await animationsAfter(page, '[data-testid="node-rubato-companion"]');
    // The data lens: particles on every declared data flow.
    counts["data lens"] = await animationsAfter(page, '.lens [role="radio"]:nth-child(2)');
    // A scenario step: the pulse.
    counts["scenario step"] = await animationsAfter(page, '.lens [role="radio"]:last-child');
    await expect(page.locator('[data-testid^="pulse-"]')).toHaveCount(1);
    const changedStep = await canvasChange(page);
    // Back to structure with nothing selected, then the issue with proposed flows: marching dashes.
    await page.getByRole("radio", { name: "Structure" }).click();
    await page.locator(".react-flow__pane").click({ position: { x: 5, y: 5 } });
    await expect(page.getByTestId("issue-row-901")).toBeVisible();
    counts["proposed flows"] = await animationsAfter(page, '[data-testid="issue-row-901"]');
    await expect(page.locator(".react-flow__edge.moving")).toHaveCount(2);
    const changedProposed = await canvasChange(page);

    console.log(`reduced motion (${run.name}): ${JSON.stringify(counts)}; pixels changed in 500 ms: ${changedStep}, ${changedProposed} (allowed ${STILL_NOISE_PX})`);
    for (const [what, c] of Object.entries(counts)) {
      expect(c, what).toEqual({ running: 0, smil: 0 });
    }
    // The still marker can be seen: no flow label, seam chip or part card covers its centre.
    // (elementFromPoint skips it: markers take no pointer events.)
    const marker = page.locator('[data-testid^="pulse-"]');
    await page.getByRole("button", { name: "← all issues" }).click();
    await page.getByRole("radio", { name: /^Scenarios/ }).click();
    const covered = await marker.evaluate((e) => {
      const r = e.getBoundingClientRect();
      const [x, y] = [r.left + r.width / 2, r.top + r.height / 2];
      return [...document.querySelectorAll(".flow-label, .seam-chip, .react-flow__node")]
        .filter((c) => {
          const b = c.getBoundingClientRect();
          return x >= b.left && x <= b.right && y >= b.top && y <= b.bottom;
        })
        .map((c) => String(c.className));
    });
    expect(covered).toEqual([]);
    expect(changedStep).toBeLessThanOrEqual(STILL_NOISE_PX);
    expect(changedProposed).toBeLessThanOrEqual(STILL_NOISE_PX);
  });
}

test("must fail: with full motion the pulse animates and the canvas moves", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await setUp(page);
  expect(await page.evaluate(() => document.documentElement.dataset["motion"])).toBe("full");
  await page.getByRole("radio", { name: /^Scenarios/ }).click();
  const pulse = page.locator('[data-testid^="pulse-"]');
  await expect(pulse).toHaveCount(1);
  expect(await pulse.evaluate((e) => e.getAnimations().length)).toBeGreaterThan(0);
  const changed = await canvasChange(page);
  console.log(`full motion: ${changed} pixels changed in 500 ms`);
  expect(changed).toBeGreaterThan(STILL_NOISE_PX);
  // The data lens: two particles per declared flow, all running.
  await page.getByRole("radio", { name: /^Data flow/ }).click();
  const running = await page.evaluate(() => document.querySelectorAll(".particle").length);
  expect(running).toBeGreaterThan(0);
});
