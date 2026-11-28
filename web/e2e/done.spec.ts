import { expect, test } from "@playwright/test";
import { openSection } from "./shell.ts";

// Issue #265: GitHub is the source. #930 is closed there (fixtures/issues.json, label
// dev:closed) and its plan sits on issue/930-closed. Pinned, it is tracked; closed, it is
// finished: in no default list, and under Done with nothing to act on.
const ISSUE = { repo: "m0x6A/rubato", number: 930 };

test("an issue closed on GitHub is listed only under Done", { tag: ["@issues.open-only", "@issues.done-view", "@plans.done-view"] }, async ({ page, baseURL }) => {
  const pinned = await page.request.post(`${baseURL}/api/t/default/w/default/pins`, { data: { issue: ISSUE, target: { kind: "node", id: "plan-check" } } });
  // Another test may have pinned it already.
  expect([200, 201, 409]).toContain(pinned.status());
  // The poll reads the issue one by one, finds it closed, and reads its plan from the branch.
  await expect
    .poll(
      async () => {
        const o = (await (await page.request.get(`${baseURL}/api/t/default/w/default/overview`)).json()) as { issues: { issue: { number: number }; closed?: boolean; finished?: boolean; plan?: unknown }[] };
        const i = o.issues.find((x) => x.issue.number === 930);
        return [i?.closed, i?.finished, Boolean(i?.plan)];
      },
      { timeout: 30_000 },
    )
    .toEqual([true, true, true]);

  await page.goto("/");
  // Not in the default list, though it is pinned.
  await expect(page.getByTestId("issues-view-open")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("issues-open")).toBeVisible();
  await expect(page.getByTestId("issue-930")).toHaveCount(0);
  // Nor on the part it is pinned to.
  await expect(page.getByTestId("node-plan-check")).not.toContainText("930");
  // Nor among the plans.
  await page.getByTestId("tab-plans").click();
  await expect(page.getByTestId("plans-proposed")).toBeVisible();
  await expect(page.getByTestId("plan-row-930")).toHaveCount(0);

  // Under Done, marked closed.
  await page.getByRole("button", { name: "Issues", exact: true }).click();
  await page.getByTestId("issues-view-done").click();
  const row = page.getByTestId("issue-930");
  await expect(row).toContainText("closed");
  await row.getByRole("button", { name: "m0x6A/rubato#930" }).click();
  const detail = page.getByTestId("issue-detail");
  await expect(detail.getByTestId("finished-note")).toContainText("Closed on GitHub.");
  // Nothing starts from here: the header offers no next step.
  await expect(detail.getByTestId("issue-next")).toHaveCount(0);
  await openSection(page, "plan");
  await expect(detail.getByTestId("where-it-fits")).toBeVisible();
  await expect(detail.getByTestId("start-session")).toHaveCount(0);
  await expect(detail.getByTestId("accept-plan")).toHaveCount(0);
});
