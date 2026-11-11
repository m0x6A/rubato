import { expect, test, type Page } from "@playwright/test";

// These run against the second server of `e2e/run.sh`: no dev user, configured as a GitHub App
// (ADR 0022), with `e2e/github-standin.ts` in place of github.com and api.github.com. So the
// browser walks the real login: redirect, consent page, code exchange, the installation lookup,
// the session cookie, and every later request through the team gate.
const github = process.env["RUBATO_E2E_GITHUB_URL"] ?? "http://127.0.0.1:18092";

/** From the login page through the stand-in's consent page, as `account`. */
async function logInAs(page: Page, account: "member" | "outsider" | "broken") {
  await page.goto("/");
  await page.getByRole("link", { name: "Log in with GitHub" }).click();
  await expect(page.getByRole("heading", { name: "GitHub stand-in" })).toBeVisible();
  return Promise.all([page.waitForResponse((r) => r.url().includes("/auth/github/callback")), page.getByRole("link", { name: `Continue as ${account}` }).click()]).then(([callback]) => callback);
}

test("without a session the app shows the login page and the API answers 401", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();
  for (const path of ["/api/me", "/api/overview", "/api/map"]) {
    expect((await page.request.get(path)).status(), path).toBe(401);
  }
});

test("a team member logs in through the App and sees what the installation lists", async ({ page }) => {
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

test("an account with no access to the installation is refused and gets no session", async ({ page }) => {
  const callback = await logInAs(page, "outsider");
  expect(callback.status()).toBe(403);
  await expect(page.getByText("has no access to the repositories this Rubato is installed on")).toBeVisible();
  expect((await page.request.get("/api/overview")).status()).toBe(401);
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();
});

test("a failed installation lookup is reported as a fault, not as no access", async ({ page }) => {
  const callback = await logInAs(page, "broken");
  expect(callback.status()).toBe(502);
  await expect(page.getByText("Could not check your access with GitHub")).toBeVisible();
  await expect(page.getByText("has no access")).toHaveCount(0);
  expect((await page.request.get("/api/me")).status()).toBe(401);
});

test("when the session ends mid-work the next write leads back to login, and nothing was written", async ({ page }) => {
  await logInAs(page, "member");
  await expect(page.getByTestId("backlog-row-911")).toBeVisible();
  // The session ends on the server (as it does when the team record is too old).
  expect((await page.request.post("/auth/logout")).status()).toBe(204);

  const [refused] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/pins") && r.request().method() === "POST"),
    page.getByTestId("backlog-row-911").dragTo(page.getByTestId("node-rubato-server")),
  ]);
  expect(refused.status()).toBe(401);
  await expect(page.getByRole("link", { name: "Log in with GitHub" })).toBeVisible();

  // Logging in again brings the work back; the refused pin left the issue not pinned.
  await page.getByRole("link", { name: "Log in with GitHub" }).click();
  await page.getByRole("link", { name: "Continue as member" }).click();
  await expect(page.getByTestId("backlog-row-911")).toBeVisible();
  await expect(page.getByTestId("issue-row-911")).toHaveCount(0);
});
