import { expect, test } from "@playwright/test";

// Issue #80: plans of voice-note issues nobody pinned are found by their `issue/` branch
// (ADR 0032), listed under Plans with where they fit, and a plan that does not fit cannot be
// accepted (ADR 0029). The dev server keeps no tokens, so Accept says why it cannot post.
test("plans from voice-note issues show where they fit", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tab-plans").click();
  const fits = page.getByTestId("plan-row-914");
  await expect(fits).toBeVisible({ timeout: 30_000 });
  await expect(fits).toContainText("fits");
  await expect(fits).toContainText("2 data models");
  await expect(fits).toContainText("found by its branch");
  const unfit = page.getByTestId("plan-row-915");
  await expect(unfit).toContainText("does not fit");
  await expect(unfit.getByRole("button", { name: "Accept plan" })).toBeDisabled();

  await fits.getByRole("button", { name: "m0x6A/rubato#914" }).click();
  const where = page.getByTestId("where-it-fits");
  await expect(where.getByTestId("fit-verdict")).toHaveText("fits the map");
  await expect(where.getByTestId("data-models")).toContainText("voice_notes");
  await expect(where.getByTestId("data-models")).toContainText("crosses web-server");
  await expect(where.getByTestId("accept-why")).toContainText("cannot comment as you");
  // Not pinned, it still shows on the parts its plan names.
  await expect(page.getByTestId("node-web")).toContainText("planned here");
});

test("connected repos list the map's repo", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tab-repos").click();
  const row = page.getByTestId("repo-m0x6A/rubato");
  await expect(row).toBeVisible();
  await expect(row).toContainText("yes");
});
