import { expect, test } from "@playwright/test";
import { openSection, showNotPinned } from "./shell.ts";

// The UX walkthrough of 2026-11-26: what a person new to Rubato tripped over, made plain.
// #915 has a plan on issue/915-voice and no test pins it, so it stays tracked and not pinned.

test("an issue with a plan but no pin is listed once, and counted once", { tag: ["@issues.listed-once"] }, async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("issue-row-915")).toContainText("not pinned");
  await showNotPinned(page);
  await expect(page.getByTestId("backlog-row-915")).toHaveCount(0);
  // The repo's folded list lists and counts every open issue once. (Its count is not compared with
  // the rack's: an issue just created through Rubato is tracked before the next GitHub poll lists
  // it, and other tests create issues in parallel.)
  await page.getByTestId("tab-repos").click();
  const work = page.getByTestId("repo-work-m0x6A/rubato");
  await page.getByTestId("repo-open-m0x6A/rubato").click();
  await expect(work.getByTestId("repo-issue-915")).toHaveCount(1);
  const ids = await work.getByTestId("repo-issues").locator("li").evaluateAll((rows) => rows.map((r) => r.getAttribute("data-testid")));
  expect(new Set(ids).size).toBe(ids.length);
  await expect(work.getByTestId("repo-work-issues")).toContainText(`${ids.length} open issue`);
});

test("Escape closes the issue, then the selection; not while typing", { tag: ["@map.escape-closes"] }, async ({ page }) => {
  // Typing in a field and pressing Escape leaves everything as it is: the list, and the seam selected.
  await page.goto("/#/issues/not-pinned?sel=seam:companion-server");
  await page.getByRole("searchbox", { name: "Filter not-pinned issues" }).fill("x");
  await page.keyboard.press("Escape");
  // (A search field clears itself on Escape; what must not happen is the view or the selection going.)
  await expect(page).toHaveURL(/#\/issues\/not-pinned\?sel=seam:companion-server$/);
  await expect(page.getByTestId("backlog")).toBeVisible();

  await page.goto("/");
  await page.getByTestId("seam-companion-server").click();
  // The inspector is the view: the issue list is not under it, and it offers the new issue.
  await expect(page.getByTestId("inspector")).toBeVisible();
  await expect(page.getByTestId("issue-list")).toHaveCount(0);
  await expect(page.getByTestId("new-issue-here")).toBeVisible();
  await expect(page.getByTestId("new-issue-start")).toHaveCount(0);
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

test("Plans from another tab shows the board; Back returns to the open issue", { tag: ["@plans.board-from-another-tab"] }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("issue-row-915").click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
  await page.getByTestId("tab-plans").click();
  await expect(page.getByTestId("plan-columns")).toBeVisible();
  await expect(page.getByTestId("issue-detail")).toHaveCount(0);
  // One view at a time (issue #357): the board took the issue's place, and Back returns to it.
  await page.goBack();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
});

test("Accept plan gives the comment to post when Rubato cannot post it", { tag: ["@plans.approve-by-comment"] }, async ({ page }) => {
  // The dev server is not a GitHub App, so it cannot comment as the person.
  await page.goto("/");
  await page.getByTestId("issue-row-914").click();
  await openSection(page, "plan");
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

test("the issue list is three lists, one at a time, each with its address", { tag: ["@issues.three-lists"] }, async ({ page }) => {
  await page.goto("/");
  const open = page.getByTestId("issues-view-open");
  const notPinned = page.getByTestId("issues-view-not-pinned");
  await expect(open).toHaveAttribute("aria-pressed", "true");
  await expect(notPinned).toHaveText(/^Not pinned \(\d+\)$/);
  await expect(page.getByTestId("issues-view-done")).toHaveText(/^Done \(\d+\)$/);
  await expect(page.getByTestId("issues-open")).toBeVisible();
  await expect(page.getByTestId("backlog")).toHaveCount(0);

  await notPinned.click();
  await expect(page).toHaveURL(/#\/issues\/not-pinned$/);
  await expect(page.getByTestId("backlog")).toBeVisible();
  await expect(page.getByTestId("issues-open")).toHaveCount(0);
  // Every row there offers its next step in sight.
  const row = page.locator('[data-testid^="backlog-row-"]').first();
  await expect(row.getByRole("button", { name: /^Pin .* to…$/ })).toBeVisible();

  // A tracked row keeps its other actions out of the way until it is pointed at or focused.
  await page.goto("/");
  const tracked = page.getByTestId("issue-912");
  const actions = tracked.locator(".row-actions");
  await page.mouse.move(0, 0);
  await expect(actions).toHaveCSS("opacity", "0");
  await tracked.hover();
  await expect(actions).toHaveCSS("opacity", "1");
  // An issue with no pin keeps Pin to… in sight: it is its next step.
  await page.mouse.move(0, 0);
  await expect(page.getByTestId("issue-915").locator(".row-actions")).toHaveCSS("opacity", "1");

  await page.getByTestId("issues-view-done").click();
  await expect(page).toHaveURL(/#\/issues\/done$/);
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/#\/issues$/);
});
