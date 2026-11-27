import { expect, test } from "@playwright/test";
import type { Me, Overview } from "../src/protocol.ts";
import { routeOverview } from "./fixtures/traffic.ts";

// Issue #80: plans of voice-note issues nobody pinned are found by their `issue/` branch
// (ADR 0032), listed under Plans with where they fit, and a plan that does not fit cannot be
// accepted (ADR 0029). The dev server keeps no tokens, so Accept says why it cannot post.
test("plans from voice-note issues show where they fit", { tag: ["@plans.found-by-branch", "@plans.unfit-cannot-be-accepted"] }, async ({ page }) => {
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
  await expect(where.getByTestId("data-models")).toContainText("crosses companion-server");
  await expect(where.getByTestId("accept-why")).toContainText("cannot comment as you");
  // Not pinned, it still shows on the parts its plan names.
  await expect(page.getByTestId("node-rubato-companion")).toContainText("plans name it");
});

// Issue #265: the Plans tab is a board. Its columns sit side by side once the panel is wide, a
// plan opens in the tab (the map still paints it), and the finished plan of #930 is only under Done.
test("the plans board: columns in a wide panel, a plan read in place, finished plans on request", { tag: ["@plans.board", "@plans.done-view"] }, async ({ page, baseURL }) => {
  await page.request.post(`${baseURL}/api/t/default/w/default/pins`, { data: { issue: { repo: "m0x6A/rubato", number: 930 }, target: { kind: "node", id: "plan-check" } } });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByTestId("tab-plans").click();
  await expect(page.getByTestId("plan-row-914")).toBeVisible({ timeout: 30_000 });
  const tops = () =>
    page.evaluate(() => ["plans-proposed", "plans-open"].map((id) => Math.round(document.querySelector(`[data-testid="${id}"]`)!.closest(".plan-column")!.getBoundingClientRect().left)));
  const [narrowProposed, narrowOpen] = await tops();
  expect(narrowOpen).toBe(narrowProposed);
  await page.getByTestId("panel-size").click();
  await expect(page.getByTestId("body")).toHaveAttribute("data-panel", "wide");
  const [wideProposed, wideOpen] = await tops();
  console.log(`plan columns: stacked at ${narrowProposed}/${narrowOpen}, side by side at ${wideProposed}/${wideOpen}`);
  expect(wideOpen!).toBeGreaterThan(wideProposed! + 200);

  // A plan opens here; the tab does not change, and the map paints the plan.
  await page.getByTestId("plan-row-914").getByRole("button", { name: "m0x6A/rubato#914" }).click();
  const detail = page.getByTestId("plan-detail");
  await expect(detail.getByTestId("where-it-fits")).toBeVisible();
  await expect(page.getByTestId("tab-plans")).toHaveClass(/active/);
  await expect(page.getByTestId("node-rubato-companion")).toHaveClass(/paint-affected/);
  await detail.getByRole("button", { name: "← all plans" }).click();
  await expect(page.getByTestId("plan-columns")).toBeVisible();

  // The finished plan is not on the board until Done is asked for, and has nothing to accept.
  await expect(page.getByTestId("plan-row-930")).toHaveCount(0);
  await page.getByTestId("plans-view-done").click();
  const done = page.getByTestId("plan-row-930");
  await expect(done).toBeVisible({ timeout: 30_000 });
  await expect(done).toContainText("closed");
  await expect(done.getByTestId("accept-plan")).toHaveCount(0);
  await done.getByRole("button", { name: "m0x6A/rubato#930" }).click();
  await expect(page.getByTestId("plan-finished-note")).toBeVisible();
  await expect(page.getByTestId("plan-detail").getByTestId("accept-plan")).toHaveCount(0);
  await page.getByTestId("plan-open-issue").click();
  await expect(page.getByTestId("issue-detail").getByTestId("finished-note")).toBeVisible();
});

test("connected repos list the map's repo", { tag: "@repos.lists-map-repo" }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tab-repos").click();
  const row = page.getByTestId("repo-m0x6A/rubato");
  await expect(row).toBeVisible();
  await expect(row).toContainText("yes");
});

// Issue #333 (ADR 0084): the e2e server runs with RUBATO_CONTEXT_GRAPH=1 and builds the graph of
// the seeded repository at start, from a real `git archive`. The Repos tab says where that graph
// stands: the head it was built at, when a tick last found it current, and its size.
test("the Repos tab says how fresh each repository's context graph is", { tag: "@repos.context-graph-age" }, async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("tab-repos").click();
  const line = page.getByTestId("graph-m0x6A/rubato");
  // The first build runs right after start; the page is told when it lands.
  await expect(line).toHaveAttribute("data-state", "current", { timeout: 60_000 });
  await expect(line).toHaveText(/^context graph: [0-9a-f]{7} · checked \d+[smh] ago · [\d,]+ nodes$/);
  // The workspace's map and pins, and the team's shared facts, each once above the repositories.
  await expect(page.getByTestId("graph-workspace/default")).toContainText(/map [0-9a-f]{7} · checked/);
  await expect(page.getByTestId("graph-team")).toContainText(/checked \d+[smh] ago/);
});

// Issue #283: Accept plan says what it did, on the row. The dev server keeps no tokens and has no
// GitHub to post to, so the page is told it may act, the post is answered here after a moment,
// and the overview then counts the approval, as it does once the comment is on the issue.
test("Accept plan reads Accepting… while it posts, and the row keeps the sentence under Open", { tag: "@plans.accept-says-so" }, async ({ page }) => {
  let accepted = false;
  await page.route("**/api/me", async (route) => {
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...((await res.json()) as Me), can_act_on_github: true } });
  });
  await routeOverview(page, (o: Overview) => ({
    ...o,
    issues: o.issues.map((i) =>
      accepted && i.issue.number === 914 ? { ...i, approval: { version: "current" as const, record: { issue: i.issue, comment_id: 1, author: "dev", sha: i.plan_sha ?? null, created_at: new Date().toISOString(), url: "https://github.invalid/c/1" } } } : i,
    ),
  }));
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/t/default/w/default/plans/approve", async (route) => {
    await held;
    accepted = true;
    await route.fulfill({ json: { state: "posted", body: "Approved: plan" } });
  });
  await page.goto("/");
  await page.getByTestId("tab-plans").click();
  const row = page.getByTestId("plan-row-914");
  await expect(page.getByTestId("plans-proposed").getByTestId("plan-row-914")).toBeVisible({ timeout: 30_000 });
  await row.getByRole("button", { name: "Accept plan" }).click();
  await expect(row.getByRole("button", { name: "Accepting…" })).toBeDisabled();
  release();
  await expect(page.getByTestId("plans-open").getByTestId("plan-row-914")).toBeVisible();
  await expect(row.getByTestId("plan-accepted")).toContainText("Accepted: posted");
  await expect(row.getByRole("button", { name: "Accept plan" })).toHaveCount(0);
});
