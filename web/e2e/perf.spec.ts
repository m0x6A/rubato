import { type Page, expect, test } from "@playwright/test";
import { bigMap } from "./fixtures/big-map.ts";

// Issue #81, design §5: a map of 40 parts and 60 seams, data lens, a working session on every
// fourth part, stays smooth: no long animation frame over 50 ms in 5 s of steady state. The
// median frame time is printed, not asserted (CI machines vary). The must-fail injects an 80 ms
// busy frame and expects it reported, so the detector itself is proven.

const WINDOW_MS = 5000;
const LONG_MS = 50;

async function openBigMap(page: Page, lens: RegExp = /^Data flow/, traffic = false): Promise<void> {
  const overview = bigMap(40, 60, traffic);
  await page.route("**/api/overview", (route) => route.fulfill({ json: overview }));
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto("/");
  await expect(page.getByTestId("node-part-00")).toBeVisible();
  await page.getByRole("radio", { name: lens }).click();
  await expect(page.locator(traffic ? ".traffic" : ".particle").first()).toBeAttached();
  // Let layout, fitView and fonts settle before measuring.
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(1000);
}

/** Long animation frames and frame times over `ms`, with `busy` ms of work injected per frame when set. */
async function measure(page: Page, ms: number, busy = 0): Promise<{ supported: boolean; long: number[]; median: number; particles: number; chevrons: number; parts: number; seams: number }> {
  return page.evaluate(
    async ({ ms, busy, longMs }) => {
      const supported = PerformanceObserver.supportedEntryTypes.includes("long-animation-frame");
      const long: number[] = [];
      const observer = supported
        ? new PerformanceObserver((list) => {
            for (const e of list.getEntries()) {
              if (e.duration > longMs) {
                long.push(Math.round(e.duration));
              }
            }
          })
        : null;
      observer?.observe({ type: "long-animation-frame", buffered: false });
      const frames: number[] = [];
      let last = performance.now();
      const end = last + ms;
      await new Promise<void>((done) => {
        const tick = (t: number) => {
          frames.push(t - last);
          last = t;
          if (busy) {
            const until = performance.now() + busy;
            while (performance.now() < until) {
              // a frame that takes too long, on purpose
            }
          }
          if (t < end) {
            requestAnimationFrame(tick);
          } else {
            done();
          }
        };
        requestAnimationFrame(tick);
      });
      // Long animation frames are reported after the frame; give the last one time to arrive.
      await new Promise((r) => setTimeout(r, 200));
      observer?.disconnect();
      const sorted = frames.slice(1).sort((a, b) => a - b);
      return {
        supported,
        long,
        median: Math.round((sorted[Math.floor(sorted.length / 2)] ?? 0) * 10) / 10,
        particles: document.querySelectorAll(".particle").length,
        chevrons: document.querySelectorAll(".traffic:not(.still)").length,
        parts: document.querySelectorAll(".react-flow__node").length,
        seams: document.querySelectorAll(".seam-chip").length,
      };
    },
    { ms, busy, longMs: LONG_MS },
  );
}

test("a map of 40 parts and 60 seams stays smooth with particles and breathing", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openBigMap(page);
  const m = await measure(page, WINDOW_MS);
  console.log(`perf: ${m.parts} parts, ${m.seams} seams, ${m.particles} particles; ${m.long.length} long frames over ${LONG_MS} ms ${JSON.stringify(m.long)}; median frame ${m.median} ms`);
  expect(m.supported, "long-animation-frame is supported, so the check can fail").toBe(true);
  expect(m.parts).toBe(40);
  expect(m.seams).toBe(60);
  expect(m.particles).toBeGreaterThan(0);
  expect(m.particles).toBeLessThanOrEqual(120);
  expect(m.long).toEqual([]);
});

test("the same map in the Traffic lens, every seam busy both ways, stays smooth at the cap (#104)", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openBigMap(page, /^Traffic/, true);
  const m = await measure(page, WINDOW_MS);
  console.log(`perf, traffic lens: ${m.parts} parts, ${m.seams} seams, ${m.chevrons} moving chevrons, ${m.particles} particles; ${m.long.length} long frames over ${LONG_MS} ms ${JSON.stringify(m.long)}; median frame ${m.median} ms`);
  expect(m.supported).toBe(true);
  expect(m.seams).toBe(60);
  expect(m.particles).toBe(0);
  expect(m.chevrons).toBe(120);
  expect(m.long).toEqual([]);
});

test("must fail: an 80 ms busy frame is reported as long", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openBigMap(page);
  const m = await measure(page, 1000, 80);
  expect(m.supported).toBe(true);
  expect(m.long.length).toBeGreaterThan(0);
});
