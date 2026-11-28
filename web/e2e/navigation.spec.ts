import { expect, test } from "@playwright/test";

// Issue #357: the URL's fragment names the view. #915 is a seeded issue with a plan
// (fixtures, branch issue/915-*); rubato-server is a part of the seeded map.
const ISSUE_915 = "#/issues/m0x6A/rubato/915";

test("reloading keeps the open issue and the selected part", { tag: ["@navigation.reload-keeps-view"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("node-rubato-server").click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  await expect(page).toHaveURL(/#\/issues\?sel=part:rubato-server$/);
  // An issue opened beside the selection: the address names both.
  await page.goto("/#/issues/m0x6A/rubato/915?sel=part:rubato-server");
  await expect(page.getByTestId("issue-detail")).toContainText("m0x6A/rubato#915");

  await page.reload();
  await expect(page.getByTestId("issue-detail")).toContainText("m0x6A/rubato#915");
  // The part is still selected: closing the issue shows its inspector again.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("issue-detail")).toHaveCount(0);
  await expect(page.getByTestId("inspector")).toContainText("rubato-server");
});

test("Back returns to the issue, then to the list; Forward goes on again", { tag: ["@navigation.back-returns"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("issue-row-915").click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
  await page.getByTestId("tab-plans").click();
  await expect(page.getByTestId("plan-columns")).toBeVisible();

  await page.goBack();
  await expect(page.getByTestId("issue-detail")).toContainText("m0x6A/rubato#915");
  await page.goBack();
  await expect(page.getByTestId("issue-detail")).toHaveCount(0);
  await expect(page.getByTestId("issue-row-915")).toBeVisible();
  await page.goForward();
  await expect(page.getByTestId("issue-detail")).toContainText("m0x6A/rubato#915");
});

test("selecting parts on the map adds no steps for Back to walk through", { tag: ["@navigation.back-returns"] }, async ({ page }) => {
  await page.goto("/#/repos");
  await expect(page.getByTestId("tab-repos")).toHaveClass(/active/);
  await page.getByRole("button", { name: /^Issues/ }).click();
  await page.getByTestId("node-rubato-server").click();
  await page.getByTestId("node-web").click();
  await expect(page.getByTestId("inspector")).toContainText("web");
  await page.goBack();
  await expect(page.getByTestId("tab-repos")).toHaveClass(/active/);
});

test("a pasted link opens the issue, or the plans", { tag: ["@navigation.link-opens-issue"] }, async ({ page }) => {
  await page.goto(`/${ISSUE_915}`);
  await expect(page.getByTestId("issue-detail")).toContainText("m0x6A/rubato#915");
  await expect(page.getByRole("button", { name: /^Issues/ })).toHaveClass(/active/);
  // A tracked issue is never reported missing.
  await expect(page.getByTestId("not-found")).toHaveCount(0);

  await page.goto("/#/plans");
  await expect(page.getByTestId("tab-plans")).toHaveClass(/active/);
  await expect(page.getByTestId("plan-columns")).toBeVisible();
});

test("a link that names nothing says so and offers the list", { tag: ["@navigation.unknown-link"] }, async ({ page }) => {
  await page.goto("/#/nowhere/at/all");
  const missing = page.getByTestId("not-found");
  await expect(missing).toContainText("Nothing is at #/nowhere/at/all");
  await missing.getByRole("button", { name: "All issues" }).click();
  await expect(missing).toHaveCount(0);
  await expect(page).toHaveURL(/#\/issues$/);

  // An issue Rubato does not track: said, with the list under it.
  await page.goto("/#/issues/m0x6A/rubato/999999");
  await expect(page.getByTestId("not-found")).toContainText("m0x6A/rubato#999999 is not among the issues Rubato tracks");
  await expect(page.getByTestId("issue-row-915")).toBeVisible();
});
