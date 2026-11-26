import { expect, test } from "@playwright/test";

// The UX walkthrough of 2026-11-26: what a person new to Rubato tripped over, made plain.
// #915 has a plan on issue/915-voice and no test pins it, so it stays tracked and not pinned.

test("an issue with a plan but no pin is listed once, and counted once", { tag: ["@issues.listed-once"] }, async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("issue-row-915")).toContainText("not pinned");
  await expect(page.getByTestId("backlog")).toBeVisible();
  await expect(page.getByTestId("backlog-row-915")).toHaveCount(0);
  // The repo's folded list counts every open issue once, as its rack unit does.
  await page.getByTestId("tab-repos").click();
  const rack = page.getByTestId("repo-m0x6A/rubato");
  const racked = Number(await rack.locator(".rack-bar.open_issues em").innerText());
  await expect(page.getByTestId("repo-work-m0x6A/rubato").locator("summary")).toContainText(`${racked} open issue`);
});

test("Escape closes the issue, then the selection; not while typing", { tag: ["@map.escape-closes"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("seam-companion-server").click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  // With a seam selected, its inspector offers the new issue; the generic button steps aside.
  await expect(page.getByTestId("new-issue-here")).toBeVisible();
  await expect(page.getByTestId("new-issue-start")).toHaveCount(0);
  // Typing in the filter and pressing Escape leaves everything open.
  await page.getByRole("searchbox", { name: "Filter not-pinned issues" }).fill("x");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("inspector")).toBeVisible();
  await page.getByRole("searchbox", { name: "Filter not-pinned issues" }).fill("");
  // An issue opened from the seam closes first, and the seam stays selected.
  await page.getByTestId("inspector").getByRole("button", { name: /#912/ }).first().click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
  await page.getByTestId("issue-detail").getByRole("heading", { level: 2 }).press("Escape");
  await expect(page.getByTestId("issue-detail")).toHaveCount(0);
  await expect(page.getByTestId("inspector")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("inspector")).toHaveCount(0);
  await expect(page.getByTestId("new-issue-start")).toBeVisible();
});

test("a new issue started with nothing selected asks where it fits", { tag: ["@new-issue.choose-place"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("new-issue-start").click();
  const form = page.getByTestId("new-issue");
  const where = form.getByLabel("Where it fits");
  await expect(where).toHaveValue("");
  await expect(where.locator("option:checked")).toHaveText("Choose a part or seam…");
  await form.getByLabel("Title").fill("A title");
  await form.getByLabel("Summary").fill("A summary.");
  await form.getByLabel("What the change does").fill("Does a thing");
  await form.getByLabel("How we'll know it works").fill("It shows");
  await form.getByLabel("What it must not break").fill("Nothing else");
  await expect(form).toContainText("Still needed: a part or seam under Where it fits.");
  await expect(form.getByRole("button", { name: /^Create in/ })).toBeDisabled();
  await where.selectOption({ label: "plan-check" });
  await expect(form).not.toContainText("Still needed");
  await form.getByRole("button", { name: "Cancel" }).click();
});

test("pairing says how, step by step, from an issue that cannot start yet", { tag: ["@companions.pairing-steps"] }, async ({ page, baseURL }) => {
  await page.goto("/");
  await page.getByTestId("issue-row-915").click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
  // map.spec.ts connects a companion as this user for a moment; while it is connected the issue
  // offers Start session instead, so the way to Companions is then its tab.
  const none = page.getByTestId("no-companion");
  if (await none.isVisible()) {
    await expect(none).toContainText("connect a companion of yours");
    await none.getByRole("button", { name: "pair one first" }).click();
  } else {
    await page.getByRole("button", { name: /^Companions/ }).click();
  }
  const panel = page.getByTestId("companions");
  await expect(panel).toContainText(`rubato-companion pair --server ${baseURL}`);
  await expect(panel.getByLabel("Code from rubato-companion pair")).toBeVisible();
  await expect(panel).toContainText("rubato-companion run");
});

test("edit mode says how to edit the map", { tag: ["@map.edit-hint"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Edit map" }).click();
  const hint = page.getByTestId("edit-hint");
  await expect(hint).toContainText("Double-click the canvas to add a part");
  await expect(hint).toContainText("draw a seam");
  await page.getByRole("button", { name: "Edit map" }).click();
  await expect(hint).toHaveCount(0);
});

test("Plans from another tab shows the board; the open issue stays open", { tag: ["@plans.board-from-another-tab"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("issue-row-915").click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
  await page.getByTestId("tab-plans").click();
  await expect(page.getByTestId("plan-columns")).toBeVisible();
  await expect(page.getByTestId("plan-detail")).toHaveCount(0);
  await page.getByRole("button", { name: /^Issues/ }).click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
});

test("Accept plan gives the comment to post when Rubato cannot post it", { tag: ["@plans.approve-by-comment"] }, async ({ page }) => {
  // The dev server is not a GitHub App, so it cannot comment as the person.
  await page.goto("/");
  await page.getByTestId("issue-row-914").click();
  const why = page.getByTestId("issue-detail").getByTestId("accept-why");
  await expect(why).toContainText("cannot comment as you");
  await expect(why).toContainText(/Instead, comment Approved: plan [0-9a-f]{10} on issue\/914-voice on the issue yourself/);
  await expect(page.getByTestId("plan-approval")).toContainText("Accept plan above, or a team member comments");
});

test("two seams between the same parts keep their chips apart", { tag: ["@map.labels-apart"] }, async ({ page }) => {
  await page.goto("/");
  // companion-claude-code and companion-mcp both join rubato-companion and claude-code.
  await expect(page.getByTestId("seam-companion-mcp")).toBeVisible();
  // Labels are placed once the cards are measured: poll until the two chips sit apart.
  await expect
    .poll(async () => {
      const a = await page.getByTestId("seam-companion-claude-code").boundingBox();
      const b = await page.getByTestId("seam-companion-mcp").boundingBox();
      return Boolean(a && b) && (a!.x + a!.width <= b!.x || b!.x + b!.width <= a!.x || a!.y + a!.height <= b!.y || b!.y + b!.height <= a!.y);
    })
    .toBe(true);
});
