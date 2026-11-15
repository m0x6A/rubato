import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { parse as parseYaml } from "yaml";

// Issue #89: the e2e server holds two workspaces (e2e/run.sh). default is the team's own map;
// shop is e2e/fixtures/shop-map.yml.
const root = resolve(import.meta.dirname, "../..");
const own = parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")) as { nodes: { id: string }[] };
const shop = parseYaml(readFileSync(resolve(import.meta.dirname, "fixtures/shop-map.yml"), "utf8")) as { nodes: { id: string }[] };

test("switch workspaces: the map, the URL and a reload follow the choice", async ({ page }) => {
  await page.goto("/");
  const select = page.getByRole("combobox", { name: "Workspace" });
  await expect(select).toHaveValue("default");
  await expect(page.getByTestId(`node-${own.nodes[0]!.id}`)).toBeVisible();

  await select.selectOption("shop");
  await expect(page).toHaveURL(/\?w=shop$/);
  for (const n of shop.nodes) {
    await expect(page.getByTestId(`node-${n.id}`)).toBeVisible();
  }
  await expect(page.getByTestId(`node-${own.nodes[0]!.id}`)).toHaveCount(0);
  await expect(page.getByText("map shop @")).toBeVisible();

  await page.reload();
  await expect(select).toHaveValue("shop");
  await expect(page.getByTestId("node-storefront")).toBeVisible();

  await select.selectOption("default");
  await expect(page.getByTestId(`node-${own.nodes[0]!.id}`)).toBeVisible();
  await expect(page.getByTestId("node-storefront")).toHaveCount(0);
});

test("a link to a workspace the server does not hold falls back to default and says so", async ({ page }) => {
  await page.goto("/?w=nope");
  await expect(page.getByText("Workspace nope no longer exists")).toBeVisible();
  await expect(page).not.toHaveURL(/w=nope/);
  await expect(page.getByTestId(`node-${own.nodes[0]!.id}`)).toBeVisible();
});

test("a part moved in shop stays where it was in default", async ({ page, request }) => {
  const base = process.env["RUBATO_E2E_URL"] ?? "http://127.0.0.1:18090";
  const put = await request.put(`${base}/api/w/shop/layout`, { headers: { "x-rubato-dev-user": "m0x6A" }, data: { positions: { storefront: { x: 40, y: 40 } } } });
  expect(put.status()).toBe(204);
  const shopOv = await (await request.get(`${base}/api/w/shop/overview`, { headers: { "x-rubato-dev-user": "m0x6A" } })).json();
  const defOv = await (await request.get(`${base}/api/overview`, { headers: { "x-rubato-dev-user": "m0x6A" } })).json();
  expect(shopOv.layout.positions.storefront).toEqual({ x: 40, y: 40 });
  expect(defOv.layout.positions.storefront).toBeUndefined();
  // Clean up: other tests and the screenshots expect shop's layout to be automatic.
  await request.delete(`${base}/api/w/shop/layout`, { headers: { "x-rubato-dev-user": "m0x6A" } });
  await page.goto("/?w=shop");
  await expect(page.getByTestId("node-storefront")).toBeVisible();
});
