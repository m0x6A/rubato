import { type Page, expect, test } from "@playwright/test";
import { freshTraffic, routeOverview, staleTraffic } from "./fixtures/traffic.ts";

// Issue #81: screenshots of every named surface, dark and light, at desktop width and at 900 px.
// Runs only with RUBATO_E2E_SHOTS=<dir>; writes <surface>-<theme>-<width>.png there and prints
// the count. Surfaces that need state the seeded server cannot make (working sessions, several
// companions) take the real overview and add it on the way to the page (design §6, A9).

const dir = process.env["RUBATO_E2E_SHOTS"];
const WIDTHS = [
  { name: "1600", width: 1600, height: 1000 },
  { name: "900", width: 900, height: 900 },
] as const;
const THEMES = ["dark", "light"] as const;

const now = () => new Date().toISOString();

/** The real overview, with working sessions on every pinned issue and three companions. */
async function atWork(page: Page): Promise<void> {
  await page.route("**/api/overview", async (route) => {
    const res = await route.fetch();
    const o = await res.json();
    const mine = { companion_id: "c-desk", hostname: "wsl-desk", user: "m0x6A", connected: true, created_at: now(), last_seen: now() };
    const mac = { companion_id: "c-mac", hostname: "mac-mini", user: "m0x6A", connected: false, created_at: now(), last_seen: new Date(Date.now() - 14 * 60_000).toISOString() };
    const mate = { companion_id: "c-mate", hostname: "teammate-laptop", user: "teammate", connected: true, created_at: now(), last_seen: new Date(Date.now() - 3 * 60_000).toISOString() };
    o.companions = [mine, mac, mate];
    let n = 0;
    for (const issue of o.issues ?? []) {
      if ((issue.pins ?? []).length === 0) {
        continue;
      }
      const status = n === 0 ? "working" : n === 1 ? "unknown" : "idle";
      issue.sessions = [{ companion_id: n === 1 ? "c-mac" : "c-desk", user: "m0x6A", cwd: `/work/rubato-${issue.issue.number}`, branch: `issue/${issue.issue.number}-x`, issue: issue.issue, repo: issue.issue.repo, source: "hook", status, last_seen: now(), spend_usd: 4.2 }];
      issue.spend_usd = 4.2 + n;
      n += 1;
    }
    await route.fulfill({ response: res, json: o });
  });
}

async function pin(page: Page, number: number, node: string): Promise<void> {
  const r = await page.request.post("/api/pins", { data: { issue: { repo: "m0x6A/rubato", number }, target: { kind: "node", id: node } } });
  expect([200, 201, 409]).toContain(r.status());
}

async function ready(page: Page): Promise<void> {
  await page.goto("/");
  await expect(page.getByTestId("seam-web-server")).toBeVisible();
}

const SURFACES: { name: string; setup: (page: Page) => Promise<void> }[] = [
  {
    name: "login",
    setup: async (page) => {
      await page.route("**/api/me", (route) => route.fulfill({ status: 401, json: { error: "not logged in" } }));
      await page.goto("/");
      await expect(page.getByRole("link", { name: /GitHub/ })).toBeVisible();
    },
  },
  {
    name: "map-data",
    setup: async (page) => {
      await ready(page);
      await page.getByRole("radio", { name: /^Data flow/ }).click();
    },
  },
  {
    name: "map-at-work",
    setup: async (page) => {
      await pin(page, 901, "rubato-server");
      await atWork(page);
      await ready(page);
      await page.getByRole("radio", { name: /^Data flow/ }).click();
    },
  },
  {
    // Issue #104: chevrons on lit seams, a hollow quiet seam and a dotted territory edge.
    name: "map-traffic",
    setup: async (page) => {
      await routeOverview(page, freshTraffic);
      await ready(page);
      await page.getByRole("radio", { name: "Traffic" }).click();
      await expect(page.getByTestId("territory-rubato-companion-github")).toBeVisible();
    },
  },
  {
    name: "map-traffic-unknown",
    setup: async (page) => {
      await routeOverview(page, staleTraffic);
      await ready(page);
      await page.getByRole("radio", { name: "Traffic" }).click();
      await expect(page.getByTestId("legend")).toContainText("Traffic unknown");
    },
  },
  {
    name: "map-plan",
    setup: async (page) => {
      await pin(page, 901, "rubato-server");
      await ready(page);
      await expect(page.getByTestId("issue-row-901")).toContainText("Designed", { timeout: 20_000 });
      await page.getByTestId("issue-row-901").click();
      await expect(page.locator(".react-flow__edge.moving")).toHaveCount(2);
    },
  },
  {
    name: "scenario-step",
    setup: async (page) => {
      await ready(page);
      await page.getByRole("radio", { name: /^Scenarios/ }).click();
      const player = page.getByRole("group", { name: "Scenario player" });
      for (let i = 0; i < 4; i++) {
        await player.getByRole("button", { name: "Next step" }).click();
      }
    },
  },
  {
    name: "inspector-part",
    setup: async (page) => {
      await ready(page);
      await page.getByTestId("node-rubato-companion").click();
      await expect(page.getByTestId("inspector")).toBeVisible();
    },
  },
  {
    name: "issue-panel",
    setup: async (page) => {
      await pin(page, 901, "rubato-server");
      await atWork(page);
      await ready(page);
      await expect(page.getByTestId("issue-row-901")).toBeVisible({ timeout: 20_000 });
      await page.getByTestId("issue-row-901").click();
      await expect(page.getByTestId("issue-detail")).toBeVisible();
    },
  },
  {
    name: "plans-tab",
    setup: async (page) => {
      await ready(page);
      await page.getByTestId("tab-plans").click();
      await expect(page.getByTestId("plan-row-914")).toBeVisible({ timeout: 30_000 });
    },
  },
  {
    name: "where-it-fits",
    setup: async (page) => {
      await ready(page);
      await page.getByTestId("tab-plans").click();
      await expect(page.getByTestId("plan-row-914")).toBeVisible({ timeout: 30_000 });
      await page.getByTestId("plan-row-914").getByRole("button", { name: "m0x6A/rubato#914" }).click();
      await expect(page.getByTestId("where-it-fits")).toBeVisible();
    },
  },
  {
    name: "connected-repos",
    setup: async (page) => {
      await ready(page);
      await page.getByTestId("tab-repos").click();
      await expect(page.getByTestId("repo-m0x6A/rubato")).toBeVisible();
    },
  },
  {
    name: "companions-lanes",
    setup: async (page) => {
      await pin(page, 901, "rubato-server");
      await atWork(page);
      await ready(page);
      await page.getByRole("button", { name: /^Companions/ }).click();
      await expect(page.getByText("wsl-desk")).toBeVisible();
    },
  },
  {
    name: "draft",
    setup: async (page) => {
      await ready(page);
      await page.getByRole("button", { name: /^Edit map/ }).click();
      await expect(page.locator(".draft-panel")).toBeVisible();
    },
  },
];

test.describe("screenshots", () => {
  test.skip(!dir, "set RUBATO_E2E_SHOTS=<dir> to write screenshots");
  let written = 0;
  for (const surface of SURFACES) {
    for (const theme of THEMES) {
      for (const size of WIDTHS) {
        test(`${surface.name} ${theme} ${size.name}`, async ({ page }) => {
          await page.emulateMedia({ colorScheme: theme });
          await page.setViewportSize({ width: size.width, height: size.height });
          await surface.setup(page);
          await page.evaluate(() => document.fonts.ready);
          // Let transitions (lens, tabs, glows: at most 480 ms) finish; endless motion keeps running.
          await page.waitForTimeout(600);
          await page.screenshot({ path: `${dir}/${surface.name}-${theme}-${size.name}.png` });
          written += 1;
        });
      }
    }
  }
  test.afterAll(() => {
    console.log(`wrote ${written} screenshots of ${SURFACES.length} surfaces (${SURFACES.length * THEMES.length * WIDTHS.length} expected) to ${dir}`);
  });
});
