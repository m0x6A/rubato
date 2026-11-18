import { execFileSync } from "node:child_process";
import { expect, test, type Page } from "@playwright/test";

// Issue #189 (ADR 0075): Configure workspace on both servers of `e2e/run.sh`.
//
// Project "settings" runs against the dev server, which holds `default` (its map read from the
// seeded repository) and `shop` (its map a file). It renames shop and moves default's map ref,
// so it runs alone, after every other project, and puts both back.
//
// Project "app" runs against the single-map server, logged in through the GitHub stand-in.
const repo = process.env["RUBATO_E2E_REPO"] ?? "";

async function openSettings(page: Page) {
  await page.getByTestId("configure-workspace").click();
  const view = page.getByTestId("settings");
  await expect(view.getByLabel("Name")).toBeVisible();
  return view;
}

async function save(page: Page) {
  const view = page.getByTestId("settings");
  await view.getByTestId("settings-save").click();
  await expect(view.getByRole("status")).toHaveText("Saved.");
}

test.describe("on the server with two workspaces", () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== "settings", "the dev server, after every other project");
  });

  test("rename shop and move default's map ref: the switcher and the map follow without a reload", { tag: ["@settings.rename", "@settings.move-map-ref", "@settings.refusal-explained"] }, async ({ page }) => {
    const sha = (branch: string) => execFileSync("git", ["-C", repo, "rev-parse", branch], { encoding: "utf8" }).trim().slice(0, 7);
    await page.goto("/?w=shop");
    // A marker a reload would lose: everything below happens on this one page load.
    await page.evaluate(() => {
      (window as unknown as { __settingsE2e: boolean }).__settingsE2e = true;
    });
    const switcher = page.getByRole("combobox", { name: "Workspace" });
    await expect(switcher.locator("option:checked")).toHaveText("Shop");

    const view = await openSettings(page);
    await expect(view.getByLabel("Name")).toHaveValue("Shop");
    await expect(view.getByText("acme/shop-map")).toBeVisible();
    try {
      // The name: every browser's switcher follows.
      await view.getByLabel("Name").fill("Corner shop");
      await save(page);
      await expect(switcher.locator("option:checked")).toHaveText("Corner shop");

      // Shop's map is a file on the server: its ref cannot move, and the view says why.
      await view.getByLabel("Ref").fill("issue/901-lens");
      await view.getByTestId("settings-save").click();
      await expect(view.getByRole("alert")).toContainText("its ref cannot move");
      expect((await (await page.request.get("/api/w/shop/settings")).json()).map_ref).toBe("main");

      // default reads its map from the seeded repository: the ref moves, and the map follows.
      await switcher.selectOption("default");
      await expect(view.getByText("m0x6A/rubato")).toBeVisible();
      await expect(view.getByLabel("Ref")).toHaveValue("main");
      await expect(page.getByText(`map rubato @ ${sha("main")}`)).toBeVisible();
      await view.getByLabel("Ref").fill("issue/901-lens");
      await save(page);
      await expect(page.getByText(`map rubato @ ${sha("issue/901-lens")}`)).toBeVisible();
      expect(await page.evaluate(() => (window as unknown as { __settingsE2e?: boolean }).__settingsE2e)).toBe(true);
    } finally {
      // Leave both workspaces as the other suites expect them.
      for (const [path, change] of [
        ["/api/settings", { map_ref: "main" }],
        ["/api/w/shop/settings", { name: "Shop" }],
      ] as const) {
        const now = await (await page.request.get(path)).json();
        const put = await page.request.put(path, { data: { ...now, ...change, traffic: null } });
        expect(put.status(), path).toBe(200);
      }
    }
    await expect(page.getByText(`map rubato @ ${sha("main")}`)).toBeVisible();
  });

  // Issue #281: a traffic source belongs to one workspace. Nothing listens on the address, so
  // shop's source fails, and default, which has none, says so and shows no state.
  test("a traffic source set in shop shows there, failing, and default still has none", { tag: ["@settings.traffic-per-workspace", "@settings.traffic-status"] }, async ({ page }) => {
    await page.goto("/?w=shop");
    const switcher = page.getByRole("combobox", { name: "Workspace" });
    const view = await openSettings(page);
    const status = view.getByTestId("settings-source-status");
    await expect(view.getByLabel("Source")).toHaveValue("");
    await expect(status).toHaveCount(0);
    try {
      await view.getByLabel("Source").selectOption("prometheus");
      await view.getByTestId("settings-field-url").fill("http://127.0.0.1:9");
      await expect(status).toHaveCount(0);
      await save(page);
      await expect(status).toHaveAttribute("data-state", "failing", { timeout: 30_000 });
      await expect(status).toContainText("Failing: ");

      await switcher.selectOption("default");
      await expect(view.getByText("m0x6A/rubato")).toBeVisible();
      await expect(view.getByLabel("Source")).toHaveValue("");
      await expect(status).toHaveCount(0);
      expect((await (await page.request.get("/api/settings")).json()).traffic ?? null).toBeNull();
      expect((await (await page.request.get("/api/w/shop/settings")).json()).traffic.fields.url).toBe("http://127.0.0.1:9");
      const overview = await (await page.request.get("/api/overview")).json();
      expect(overview.traffic?.state ?? "not_configured").toBe("not_configured");
    } finally {
      const now = await (await page.request.get("/api/w/shop/settings")).json();
      const put = await page.request.put("/api/w/shop/settings", { data: { ...now, traffic: null } });
      expect(put.status()).toBe(200);
    }
    await view.getByRole("button", { name: "Close" }).click();
    await page.getByRole("radio", { name: "Traffic" }).click();
    await expect(page.getByText("This workspace has no traffic source. Set one in Configure workspace.")).toBeVisible();
  });
});

test.describe("on the server with one workspace", () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== "app", "the App server");
  });

  test("the header item opens the settings of the one workspace, and nothing is created", { tag: "@settings.single-workspace" }, async ({ page }) => {
    await page.goto("/");
    await page.getByRole("link", { name: "Log in with GitHub" }).click();
    await page.getByRole("link", { name: "Continue as member" }).click();
    await expect(page.getByTestId("node-rubato-server")).toBeVisible();
    const before = await (await page.request.get("/api/workspaces")).json();
    expect(before.map((w: { id: string }) => w.id)).toEqual(["default"]);

    await expect(page.getByRole("combobox", { name: "Workspace" })).toHaveCount(0);
    const view = await openSettings(page);
    await expect(view.getByText("m0x6A/rubato")).toBeVisible();
    await expect(view.getByLabel("Ref")).toHaveValue("main");
    await expect(view.getByLabel("Source")).toHaveValue("");
    // default cannot be deleted, so the view does not offer it.
    await expect(view.getByRole("button", { name: "Delete this workspace…" })).toHaveCount(0);
    await expect(view.getByRole("button", { name: "New workspace…" })).toBeVisible();

    await view.getByRole("button", { name: "Close" }).click();
    await expect(page.getByTestId("settings")).toHaveCount(0);
    expect(await (await page.request.get("/api/workspaces")).json()).toEqual(before);
  });
});
