import { expect, type Page } from "@playwright/test";

// The top bar's two menus (issue #357), as the specs use them.

/** The workspace menu's button: it names the workspace shown. */
export const workspaceButton = (page: Page) => page.getByTestId("workspace-menu").getByRole("button").first();

/** Show another workspace through the workspace menu. */
export async function switchWorkspace(page: Page, id: string) {
  await workspaceButton(page).click();
  await page.getByTestId(`switch-workspace-${id}`).click();
}

/** The workspace menu says which map it shows and at which commit: `map <name> @ <sha7>`. */
export async function expectMapVersion(page: Page, text: string | RegExp) {
  await expect(async () => {
    if ((await page.getByTestId("map-version").count()) === 0) {
      await workspaceButton(page).click();
    }
    await expect(page.getByTestId("map-version")).toContainText(text, { timeout: 1000 });
  }).toPass({ timeout: 15_000 });
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("map-version")).toHaveCount(0);
}

/** Open a page from the account menu: `open-members`, `open-operator` or `log-out`. */
export async function account(page: Page, item: "open-members" | "open-operator" | "log-out") {
  await page.getByTestId("account-menu").getByRole("button").first().click();
  await page.getByTestId(item).click();
}

/** The address of the issue list's Not pinned segment: open issues of the map's repos nobody tracks yet. */
export const NOT_PINNED = "/#/issues/not-pinned";

/** Show the Not pinned segment of the issue list. */
export async function showNotPinned(page: Page) {
  await page.getByTestId("issues-view-not-pinned").click();
  await expect(page.getByTestId("backlog")).toBeVisible();
}

/** Show one part of the open issue: its plan, its room, its sessions or its pull request. */
export async function openSection(page: Page, section: "overview" | "plan" | "room" | "sessions" | "pr") {
  await page.getByTestId("issue-detail").getByTestId(section === "pr" ? "open-pull-view" : `section-${section}`).click();
  await expect(page.locator(`[data-testid="issue-detail"] [data-section="${section}"]`)).toBeVisible();
}

/** The map's View menu: how the map is laid out, and the two ways to change it. */
async function openViewMenu(page: Page) {
  await page.getByTestId("view-menu").getByRole("button").first().click();
}

/** Turn By repo on or off, through the View menu. */
export async function toggleByRepo(page: Page) {
  await openViewMenu(page);
  await page.getByTestId("view-by-repo").click();
}

/** Auto-arrange the layout that is showing, for everyone, through the View menu. */
export async function autoArrange(page: Page) {
  await openViewMenu(page);
  await page.getByTestId("view-auto-arrange").click();
}

/** Open map versus territory from the Traffic lens's line: what was observed and the map does not explain. */
export async function openTerritory(page: Page) {
  await page.getByTestId("to-settle").click();
  await expect(page.getByTestId("territory")).toBeVisible();
}

/** Choose something from the map draft's More menu: "Copy YAML" or "Discard draft". */
export async function draftMore(page: Page, item: "Copy YAML" | "Discard draft") {
  await page.getByTestId("draft-more").getByRole("button").first().click();
  await page.getByTestId("draft-more").getByRole("menuitem", { name: item }).click();
}
