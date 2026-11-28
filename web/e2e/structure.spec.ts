import { expect, test } from "@playwright/test";
import { workspaceButton } from "./shell.ts";

// Issue #357: what every view has in common, checked on one address of each kind. Later PRs of
// the stack add their screens' rules here (one primary action, the map band as one row).

/** One address per panel view, and what its view root is called. */
const PANEL: [fragment: string, view: string][] = [
  ["#/issues", "issues"],
  ["#/issues/not-pinned", "issues"],
  ["#/issues/done", "issues"],
  ["#/map?sel=part:web", "map"],
  ["#/issues/m0x6A/rubato/915", "issue"],
  ["#/issues/new", "new-issue"],
  ["#/plans", "plans"],
  ["#/plans/done", "plans"],
  // A plan's address from before it became a part of its issue still opens it.
  ["#/plans/m0x6A/rubato/915", "issue"],
  ["#/issues/m0x6A/rubato/915/plan?from=plans", "issue"],
  ["#/companions", "companions"],
  ["#/repos", "repos"],
  ["#/repos/m0x6A/rubato", "repo"],
  ["#/pulls/m0x6A/rubato/917", "pull"],
  ["#/findings", "findings"],
  ["#/nowhere", "not-found"],
];

/** The pages: the map gives way, the top bar stays. */
const PAGES: [fragment: string, view: string][] = [
  ["#/settings", "settings"],
  ["#/settings/workspaces", "workspaces"],
  ["#/members", "members"],
];

/** How many view roots the page shows, and whether a findings list sits outside the view. */
async function shape(page: import("@playwright/test").Page) {
  return page.evaluate(() => ({
    roots: [...document.querySelectorAll("[data-view]")].map((e) => e.getAttribute("data-view")),
    strayFindings: [...document.querySelectorAll('[data-testid="findings"]')].filter((e) => !e.closest('[data-view="findings"]')).length,
    topBar: document.querySelectorAll('.topbar .brand > *, .topbar .status > *:not([data-testid="live-words"])').length,
  }));
}

test("every address shows one view, with nothing stacked above it, under a top bar of four", { tag: ["@navigation.one-view", "@findings.own-view"] }, async ({ page }) => {
  let checked = 0;
  for (const [fragment, view] of PANEL) {
    await page.goto(`/${fragment}`);
    await expect(page.locator(`.panel [data-view="${view}"]`), fragment).toBeVisible();
    // The two menus arrive with who the person is and what the workspace holds.
    await expect(workspaceButton(page), fragment).toBeVisible();
    await expect(page.getByTestId("account-menu"), fragment).toBeVisible();
    const s = await shape(page);
    expect(s.roots, fragment).toEqual([view]);
    expect(s.strayFindings, fragment).toBe(0);
    expect(s.topBar, fragment).toBe(4);
    await expect(page.locator("section.map"), fragment).toBeVisible();
    checked += 1;
  }
  for (const [fragment, view] of PAGES) {
    await page.goto(`/${fragment}`);
    await expect(page.locator(`main.page[data-view="${view}"]`), fragment).toBeVisible();
    await expect(page.getByTestId("account-menu"), fragment).toBeVisible();
    await expect(workspaceButton(page), fragment).toBeVisible();
    const s = await shape(page);
    expect(s.roots, fragment).toEqual([view]);
    expect(s.topBar, fragment).toBe(4);
    await expect(page.getByTestId("body"), fragment).toHaveAttribute("data-page", view);
    // Hidden, not gone: the map keeps its lens and its positions for when the page closes.
    await expect(page.locator("section.map"), fragment).toBeHidden();
    await expect(page.locator("section.map"), fragment).toHaveCount(1);
    await expect(page.locator("aside.panel"), fragment).toHaveCount(0);
    await expect(workspaceButton(page), fragment).toBeVisible();
    checked += 1;
  }
  console.log(`checked ${checked} addresses: one view each, a top bar of four`);
  expect(checked).toBe(PANEL.length + PAGES.length);
});

test("must fail: a second view root, or a findings list outside its view, is caught", { tag: ["@navigation.one-view", "@selftest"] }, async ({ page }) => {
  await page.goto("/#/issues");
  await expect(page.locator('.panel [data-view="issues"]')).toBeVisible();
  await page.evaluate(() => {
    const extra = document.createElement("div");
    extra.setAttribute("data-view", "plans");
    const findings = document.createElement("div");
    findings.setAttribute("data-testid", "findings");
    document.querySelector(".panel")?.prepend(extra, findings);
  });
  const s = await shape(page);
  expect(s.roots).toEqual(["plans", "issues"]);
  expect(s.strayFindings).toBe(1);
});

test("findings have their own view: a quiet link in the panel, the list one step away", { tag: ["@findings.own-view"] }, async ({ page }) => {
  const overview = await (await page.request.get("/api/t/default/w/default/overview")).json();
  const n = (overview.findings ?? []).length as number;
  await page.goto("/");
  const link = page.getByTestId("findings-link");
  if (n === 0) {
    await expect(link).toHaveCount(0);
    await page.goto("/#/findings");
    await expect(page.locator('[data-view="findings"]')).toContainText("Nothing: the map, the plans and what is observed agree.");
    return;
  }
  await expect(link).toHaveText(new RegExp(`^${n} findings?$`));
  await expect(page.getByTestId("findings")).toHaveCount(0);
  await link.click();
  await expect(page).toHaveURL(/#\/findings$/);
  await expect(page.getByTestId("findings")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("findings")).toHaveCount(0);
  await expect(link).toBeVisible();
});
