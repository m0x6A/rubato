import { execFileSync } from "node:child_process";
import { expect, test, type Page } from "@playwright/test";
import { expectMapVersion, switchWorkspace, workspaceButton } from "./shell.ts";

// Issue #189 (ADR 0075): Configure workspace on both servers of `e2e/run.sh`.
//
// Project "settings" runs against the dev server, which holds `default` (its map read from the
// seeded repository) and `shop` (its map a file). It renames shop and moves default's map ref,
// so it runs alone, after every other project, and puts both back.
//
// Project "app" runs against the single-map server, logged in through the GitHub stand-in.
const repo = process.env["RUBATO_E2E_REPO"] ?? "";

async function openSettings(page: Page) {
  await workspaceButton(page).click();
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
    await expect(workspaceButton(page)).toContainText("Shop");

    const view = await openSettings(page);
    await expect(view.getByLabel("Name")).toHaveValue("Shop");
    await expect(view.getByText("acme/shop-map")).toBeVisible();
    try {
      // The name: every browser's switcher follows.
      await view.getByLabel("Name").fill("Corner shop");
      await save(page);
      await expect(workspaceButton(page)).toContainText("Corner shop");

      // Shop's map is a file on the server: its ref cannot move, and the view says why.
      await view.getByLabel("Ref").fill("issue/901-lens");
      await view.getByTestId("settings-save").click();
      await expect(view.getByRole("alert")).toContainText("its ref cannot move");
      expect((await (await page.request.get("/api/t/default/w/shop/settings")).json()).map_ref).toBe("main");

      // default reads its map from the seeded repository: the ref moves, and the map follows.
      await switchWorkspace(page, "default");
      await expect(view.getByText("m0x6A/rubato")).toBeVisible();
      await expect(view.getByLabel("Ref")).toHaveValue("main");
      await expectMapVersion(page, `map rubato @ ${sha("main")}`);
      await view.getByLabel("Ref").fill("issue/901-lens");
      await save(page);
      await expectMapVersion(page, `map rubato @ ${sha("issue/901-lens")}`);
      expect(await page.evaluate(() => (window as unknown as { __settingsE2e?: boolean }).__settingsE2e)).toBe(true);
    } finally {
      // Leave both workspaces as the other suites expect them.
      for (const [path, change] of [
        ["/api/t/default/w/default/settings", { map_ref: "main" }],
        ["/api/t/default/w/shop/settings", { name: "Shop" }],
      ] as const) {
        const now = await (await page.request.get(path)).json();
        const put = await page.request.put(path, { data: { ...now, ...change, traffic: null } });
        expect(put.status(), path).toBe(200);
      }
    }
    await expectMapVersion(page, `map rubato @ ${sha("main")}`);
  });

  // Issue #281: a traffic source belongs to one workspace. Nothing listens on the address, so
  // shop's source fails, and default, which has none, says so and shows no state.
  test("a traffic source set in shop shows there, failing, and default still has none", { tag: ["@settings.traffic-per-workspace", "@settings.traffic-status"] }, async ({ page }) => {
    await page.goto("/?w=shop");
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

      await switchWorkspace(page, "default");
      await expect(view.getByText("m0x6A/rubato")).toBeVisible();
      await expect(view.getByLabel("Source")).toHaveValue("");
      await expect(status).toHaveCount(0);
      expect((await (await page.request.get("/api/t/default/w/default/settings")).json()).traffic ?? null).toBeNull();
      expect((await (await page.request.get("/api/t/default/w/shop/settings")).json()).traffic.fields.url).toBe("http://127.0.0.1:9");
      const overview = await (await page.request.get("/api/t/default/w/default/overview")).json();
      expect(overview.traffic?.state ?? "not_configured").toBe("not_configured");
    } finally {
      const now = await (await page.request.get("/api/t/default/w/shop/settings")).json();
      const put = await page.request.put("/api/t/default/w/shop/settings", { data: { ...now, traffic: null } });
      expect(put.status()).toBe(200);
    }
    await view.getByRole("button", { name: "Close" }).click();
    await page.getByRole("radio", { name: "Traffic" }).click();
    await expect(page.getByText("This workspace has no traffic source. Set one in Configure workspace.")).toBeVisible();
  });

  // Issue #288: the namespaces a source reads are a field of the adapter, drawn from its list.
  test("a source keeps the namespaces it lists, and refuses a list that is not namespaces", { tag: "@settings.traffic-namespaces" }, async ({ page }) => {
    await page.goto("/?w=shop");
    const view = await openSettings(page);
    try {
      await view.getByLabel("Source").selectOption("prometheus");
      await view.getByTestId("settings-field-url").fill("http://127.0.0.1:9");
      const namespaces = view.getByTestId("settings-field-namespaces");
      await expect(namespaces).toHaveAttribute("placeholder", "rubato, shop");
      await namespaces.fill("shop|.*");
      await view.getByTestId("settings-save").click();
      await expect(view.getByRole("alert")).toContainText("is not a namespace");
      expect((await (await page.request.get("/api/t/default/w/shop/settings")).json()).traffic ?? null).toBeNull();
      await namespaces.fill("rubato, shop");
      await save(page);
      expect((await (await page.request.get("/api/t/default/w/shop/settings")).json()).traffic.fields.namespaces).toBe("rubato, shop");
    } finally {
      const now = await (await page.request.get("/api/t/default/w/shop/settings")).json();
      const put = await page.request.put("/api/t/default/w/shop/settings", { data: { ...now, traffic: null } });
      expect(put.status()).toBe(200);
    }
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
    const before = await (await page.request.get("/api/t/default/workspaces")).json();
    expect(before.map((w: { id: string }) => w.id)).toEqual(["default"]);

    // One workspace: the menu offers none to switch to, and still leads to the settings.
    await workspaceButton(page).click();
    await expect(page.getByRole("menuitemradio")).toHaveCount(0);
    await page.keyboard.press("Escape");
    const view = await openSettings(page);
    // A page of its own: the map is hidden, not the top bar.
    await expect(page.getByTestId("body")).toHaveAttribute("data-page", "settings");
    await expect(page.getByTestId("node-rubato-server")).toBeHidden();
    await expect(view.getByText("m0x6A/rubato")).toBeVisible();
    await expect(view.getByLabel("Ref")).toHaveValue("main");
    await expect(view.getByLabel("Source")).toHaveValue("");
    // Creating and deleting workspaces are not this page's job.
    await expect(view.getByRole("button", { name: /Delete|New workspace/ })).toHaveCount(0);

    await view.getByRole("button", { name: "Close" }).click();
    await expect(page.getByTestId("settings")).toHaveCount(0);
    await expect(page.getByTestId("node-rubato-server")).toBeVisible();

    // The Workspaces page lists the one workspace; default cannot be deleted, so it is not offered.
    await workspaceButton(page).click();
    await page.getByTestId("open-workspaces").click();
    const list = page.getByTestId("workspaces");
    await expect(list.getByTestId("workspace-default")).toContainText("showing now");
    await expect(list.getByRole("button", { name: /^Delete/ })).toHaveCount(0);
    await expect(list.getByRole("button", { name: "New workspace…" })).toBeVisible();
    await list.getByRole("button", { name: "Close" }).click();
    await expect(list).toHaveCount(0);
    expect(await (await page.request.get("/api/t/default/workspaces")).json()).toEqual(before);
  });
});
