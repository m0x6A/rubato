import { execFileSync } from "node:child_process";
import { expect, test } from "@playwright/test";

// Issue #82 on the dev server: the seeded pull request #917 (RUBATO_DEV_PULLS) from issue/917-pull
// into e2e/pull-base lists on the Repos tab, opens its view, and merges into the bare repository
// as the dev user. Its base is a branch of its own, so main (where the map is read) never moves.
const repo = process.env["RUBATO_E2E_REPO"] ?? "";
const head = (branch: string) => execFileSync("git", ["-C", repo, "rev-parse", branch], { encoding: "utf8" }).trim();

test("a pull request lists on the Repos tab, opens, and merges into the repository", { tag: ["@pulls.listed-per-repo", "@pulls.view", "@pulls.merge-as-yourself", "@pulls.merge-refused-says-why"] }, async ({ page }) => {
  const before = head("e2e/pull-base");
  const pullHead = head("issue/917-pull");
  await page.goto("/");
  // The listing arrives with a poll tick.
  await expect.poll(async () => JSON.stringify((await (await page.request.get("/api/t/default/w/default/overview")).json()).pulls ?? []), { timeout: 20_000 }).toContain('"number":917');
  await page.getByTestId("tab-repos").click();
  const work = page.getByTestId("repo-work-m0x6A/rubato");
  await page.getByTestId("repo-open-m0x6A/rubato").click();
  await expect(work).toContainText("open pull request");
  await work.getByTestId("pull-row-917").getByRole("button").click();
  // A pull request has a view and an address of its own; its way back is its repo.
  await expect(page).toHaveURL(/#\/pulls\/m0x6A\/rubato\/917$/);
  await expect(page.getByRole("button", { name: "← m0x6A/rubato" })).toBeVisible();
  const view = page.getByTestId("pull-view");
  await expect(view).toContainText("A change to merge from Rubato");
  await expect(view.getByTestId("pull-files")).toContainText("1 file");
  await expect(view).toContainText("no checks");
  const merge = view.getByRole("button", { name: "Merge" });
  await expect(merge).toBeEnabled();
  await view.getByRole("combobox").selectOption("merge");
  await merge.click();
  await expect(view.getByTestId("merge-result")).toHaveText("Merged by m0x6A.");
  // A merge commit on the base, with the pull request's head as its second parent.
  const after = head("e2e/pull-base");
  expect(after).not.toBe(before);
  expect(execFileSync("git", ["-C", repo, "rev-parse", `${after}^2`], { encoding: "utf8" }).trim()).toBe(pullHead);
  expect(head("main")).not.toBe(after);
  // The view reads again and says it merged; Merge is off.
  await expect(view).toContainText("merged");
  await expect(view.getByTestId("merge-why-not")).toHaveText("It is not open.");
});
