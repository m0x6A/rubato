import { expect, test } from "@playwright/test";

// Issue #357: the URL's fragment names the view. #915 is a seeded issue with a plan
// (fixtures, branch issue/915-*); rubato-server is a part of the seeded map.
const ISSUE_915 = "#/issues/m0x6A/rubato/915";

test("reloading keeps the open issue and the selected part", { tag: ["@navigation.reload-keeps-view"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("node-rubato-server").click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  await expect(page).toHaveURL(/#\/map\?sel=part:rubato-server$/);
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
  // The first selection opens the inspector, one step; another part takes its place.
  await page.getByTestId("node-rubato-server").click();
  await expect(page.getByTestId("inspector")).toContainText("rubato-server");
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

test("a link opens one part of an issue; its parts are one step for Back", { tag: ["@navigation.link-opens-issue-section", "@issues.parts"] }, async ({ page }) => {
  await page.goto("/#/plans");
  await expect(page.getByTestId("plan-columns")).toBeVisible();
  await page.goto(`/${ISSUE_915}/plan`);
  const detail = page.getByTestId("issue-detail");
  await expect(detail.getByTestId("section-plan")).toHaveAttribute("aria-current", "page");
  await expect(detail.getByTestId("where-it-fits")).toBeVisible();
  // The other parts are there, not showing.
  await expect(detail.locator('[data-section="overview"]')).toBeHidden();

  await detail.getByTestId("section-room").click();
  await expect(page).toHaveURL(/#\/issues\/m0x6A\/rubato\/915\/room$/);
  await detail.getByTestId("section-sessions").click();
  await expect(detail.getByTestId("no-sessions")).toBeVisible();
  await detail.getByTestId("section-overview").click();
  await expect(page).toHaveURL(/#\/issues\/m0x6A\/rubato\/915$/);
  await expect(detail.locator('[data-section="overview"]')).toBeVisible();
  // One step back leaves the issue, whatever parts were looked at.
  await page.goBack();
  await expect(page.getByTestId("plan-columns")).toBeVisible();

  // An issue with no pull request has no such part; a link to it shows the overview.
  await page.goto(`/${ISSUE_915}/pr`);
  await expect(detail.getByTestId("open-pull-view")).toHaveCount(0);
  await expect(detail.locator('[data-section="overview"]')).toBeVisible();
});

test("what was typed in one part of an issue is there on coming back", { tag: ["@issues.parts"] }, async ({ page }) => {
  await page.goto(`/${ISSUE_915}/room`);
  const box = page.getByLabel("Message to the room");
  await box.fill("half a thought");
  await page.getByTestId("section-plan").click();
  await expect(box).toBeHidden();
  await page.getByTestId("section-room").click();
  await expect(box).toHaveValue("half a thought");
});

test("an issue's header offers one next step for where it stands", { tag: ["@issues.next-action"] }, async ({ page }) => {
  // #915 has a plan and no pin, and the dev user has no companion here: not on the map yet.
  await page.goto(`/${ISSUE_915}`);
  const next = page.getByTestId("issue-next");
  await expect(next).toHaveAttribute("data-next", "pin");
  await expect(next).toContainText("not on the map yet");
  await expect(next.getByTestId("next-step").getByRole("button", { name: "Pin m0x6A/rubato#915 to…" })).toBeVisible();
  await expect(page.locator('[data-testid="issue-detail"] .issue-head').locator('.primary, [data-testid="next-step"]')).toHaveCount(1);

  // #912 is pinned by its Map: line; with no companion paired, pairing one is the step.
  await page.goto("/#/issues/m0x6A/rubato/912");
  await expect(next).toHaveAttribute("data-next", "pair");
  await next.getByRole("button", { name: "Pair a companion" }).click();
  await expect(page).toHaveURL(/#\/companions$/);
});
