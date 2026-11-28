import { expect, test, type Page } from "@playwright/test";
import { account } from "./shell.ts";

// These run against the second server of `e2e/run.sh`: no dev user, configured as a GitHub App
// (ADR 0022), with `e2e/github-standin.ts` in place of github.com and api.github.com. So the
// browser walks the real login: redirect, consent page, code exchange, the invite gate (ADR 0088:
// `run.sh` makes `member` a member of tenant `default`), the session cookie, and every later
// request through the membership layer.
const github = process.env["RUBATO_E2E_GITHUB_URL"] ?? "http://127.0.0.1:18092";

/** From the login page through the stand-in's consent page, as `account`. */
async function logInAs(page: Page, account: "member" | "outsider") {
  await page.goto("/");
  await page.getByRole("link", { name: "Log in with GitHub" }).click();
  await expect(page.getByRole("heading", { name: "GitHub stand-in" })).toBeVisible();
  return Promise.all([page.waitForResponse((r) => r.url().includes("/auth/github/callback")), page.getByRole("link", { name: `Continue as ${account}` }).click()]).then(([callback]) => callback);
}

test("without a session the app shows the login page and the API answers 401", { tag: "@auth.login-page" }, async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();
  for (const path of ["/api/me", "/api/t/default/w/default/overview", "/api/t/default/w/default/map"]) {
    expect((await page.request.get(path)).status(), path).toBe(401);
  }
});

test("a team member logs in through the App and sees what the installation lists", { tag: "@auth.member-logs-in" }, async ({ page }) => {
  const callback = await logInAs(page, "member");
  expect(callback.status()).toBe(303);
  await expect(page.getByTestId("node-rubato-server")).toBeVisible();
  const me = await page.request.get("/api/me");
  expect(me.status()).toBe(200);
  expect((await me.json()).login).toBe("member");

  // The open issue reached the page through the App's key, an installation token and the
  // issue listing; the pull request in the same listing is left out.
  const backlog = page.getByTestId("backlog");
  await expect(backlog.getByTestId("backlog-row-911")).toContainText("Listed as the App installation");
  await expect(backlog.getByTestId("backlog-row-912")).toHaveCount(0);
  await expect(backlog.getByTestId("backlog-repo-m0x6A/rubato")).not.toHaveClass(/warn/);

  // The server acted as the App: it checked itself and minted an installation token.
  const calls = (await (await fetch(`${github}/_standin/calls`)).json()) as Record<string, number>;
  expect(calls["/app"]).toBeGreaterThan(0);
  expect(calls["/app/installations/34/access_tokens"]).toBeGreaterThan(0);
  expect(calls["/repos/m0x6A/rubato/issues"]).toBeGreaterThan(0);

  // The session survives a reload.
  await page.reload();
  await expect(page.getByTestId("node-rubato-server")).toBeVisible();
});

test("a person logs out from the account menu: the login page shows and the session no longer works", { tag: "@auth.log-out" }, async ({ page }) => {
  await logInAs(page, "member");
  await expect(page.getByTestId("node-rubato-server")).toBeVisible();
  expect((await page.request.get("/api/me")).status()).toBe(200);
  await account(page, "log-out");
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();
  expect((await page.request.get("/api/me")).status()).toBe(401);
});

test("logging in returns to the view the link named", { tag: "@navigation.login-returns" }, async ({ page }) => {
  await page.goto("/#/plans");
  await page.getByRole("link", { name: "Log in with GitHub" }).click();
  await expect(page.getByRole("heading", { name: "GitHub stand-in" })).toBeVisible();
  await Promise.all([page.waitForResponse((r) => r.url().includes("/auth/github/callback")), page.getByRole("link", { name: "Continue as member" }).click()]);
  // The callback returns to the page with no fragment; the view asked for is restored.
  await expect(page.getByTestId("tab-plans")).toHaveClass(/active/);
  await expect(page).toHaveURL(/#\/plans$/);
});

test("a member creates an issue from a part, as the App, and it is pinned there (ADR 0025)", { tag: ["@new-issue.pinned-where-begun", "@new-issue.created-as-the-app"] }, async ({ page }) => {
  await logInAs(page, "member");
  // The server read the installation's permissions as the App on its first full pass.
  await expect.poll(async () => (await (await page.request.get("/api/t/default/w/default/overview")).json()).can_create_issues, { timeout: 15_000 }).toBe(true);
  await page.reload();
  await page.getByTestId("node-rubato-server").click();
  await page.getByTestId("inspector").getByTestId("new-issue-here").click();
  const form = page.getByTestId("new-issue");
  await form.getByLabel("Title").fill("Created through the App");
  await form.getByLabel("Summary").fill("An issue written in Rubato.");
  await form.getByLabel("What the change does").fill("Exists");
  await form.getByLabel("How we'll know it works").fill("The stand-in got one POST");
  await form.getByLabel("What it must not break").fill("Nothing");
  await form.getByRole("button", { name: "Create in m0x6A/rubato" }).click();
  const detail = page.getByTestId("issue-detail");
  await expect(detail).toContainText("m0x6A/rubato#950");
  await expect(detail).toContainText("node rubato-server");

  const calls = (await (await fetch(`${github}/_standin/calls`)).json()) as Record<string, number>;
  expect(calls["/app/installations/34"]).toBeGreaterThan(0);
});

test("a member merges a pull request as themselves through their token (ADR 0033)", { tag: ["@pulls.view", "@pulls.merge-as-yourself"] }, async ({ page }) => {
  await logInAs(page, "member");
  await expect.poll(async () => JSON.stringify((await (await page.request.get("/api/t/default/w/default/overview")).json()).pulls ?? []), { timeout: 20_000 }).toContain('"number":920');
  await page.reload();
  await page.getByTestId("tab-repos").click();
  const work = page.getByTestId("repo-work-m0x6A/rubato");
  await work.locator("summary").click();
  await work.getByTestId("pull-row-920").getByRole("button").click();
  const view = page.getByTestId("pull-view");
  await expect(view).toContainText("Merges as the person who pressed Merge.");
  await expect(view).toContainText("checks green");
  // The repo allows merge commits and squash, not rebase: only those two are offered, squash first.
  const select = view.getByRole("combobox");
  await expect(select).toHaveValue("squash");
  await expect(select.locator("option")).toHaveText(["Merge commit", "Squash and merge"]);
  await view.getByRole("button", { name: "Merge" }).click();
  await expect(view.getByTestId("merge-result")).toHaveText("Merged by member.");
  // GitHub recorded the person, at the head they saw, with the method they picked.
  const merges = (await (await fetch(`${github}/_standin/merges`)).json()) as { login: string; number: number; sha: string; merge_method: string }[];
  expect(merges).toEqual([{ login: "member", number: 920, sha: "a920a920a920a920a920a920a920a920a920a920", merge_method: "squash" }]);
});

test("an account nobody invited is refused and gets no session", { tag: "@auth.no-access-refused" }, async ({ page }) => {
  const callback = await logInAs(page, "outsider");
  expect(callback.status()).toBe(403);
  await expect(page.getByText("You have not been invited to a tenant on this Rubato")).toBeVisible();
  expect((await page.request.get("/api/t/default/w/default/overview")).status()).toBe(401);
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();
});

test("when the session ends mid-work the next write leads back to login, and nothing was written", { tag: "@auth.session-ends-mid-work" }, async ({ page }) => {
  await logInAs(page, "member");
  await expect(page.getByTestId("backlog-row-911")).toBeVisible();
  // The app can also find out by itself: a poll event makes it read the overview, which now
  // answers 401, and the login page replaces the row before the drag. Hold the app's own reads
  // until the write has answered, so the write is what finds out.
  let release = () => {};
  const held = new Promise<void>((done) => (release = done));
  await page.route(/\/api\/(t\/[^/]+\/w\/[^/]+\/overview|me)(\?|$)/, async (route) => {
    await held;
    await route.continue().catch(() => {});
  });
  // The session ends on the server (as it does when the team record is too old).
  expect((await page.request.post("/auth/logout")).status()).toBe(204);

  const [refused] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/t/default/w/default/pins") && r.request().method() === "POST"),
    page.getByTestId("backlog-row-911").dragTo(page.getByTestId("node-rubato-server")),
  ]);
  expect(refused.status()).toBe(401);
  release();
  await page.unroute(/\/api\/(t\/[^/]+\/w\/[^/]+\/overview|me)(\?|$)/);
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();

  // Logging in again brings the work back; the refused pin left the issue not pinned.
  await page.getByRole("link", { name: "Log in with GitHub" }).click();
  await page.getByRole("link", { name: "Continue as member" }).click();
  await expect(page.getByTestId("backlog-row-911")).toBeVisible();
  await expect(page.getByTestId("issue-row-911")).toHaveCount(0);
});
