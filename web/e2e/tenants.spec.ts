import { expect, test, type Page } from "@playwright/test";
import { account } from "./shell.ts";

// Plan 345 (ADR 0087, 0088): members and the operator's page, against the second server of
// `e2e/run.sh`: a GitHub App with `e2e/github-standin.ts` for GitHub. `member` is an admin of
// tenant `default` and the server's operator; `newcomer` has a GitHub account and no invite.

async function logIn(page: Page) {
  await page.goto("/");
  await page.getByRole("link", { name: "Log in with GitHub" }).click();
  await Promise.all([page.waitForResponse((r) => r.url().includes("/auth/github/callback")), page.getByRole("link", { name: "Continue as member" }).click()]);
  await expect(page.getByTestId("node-rubato-server")).toBeVisible();
}

test("an admin invites a GitHub login, sees it as invited, and removes it after a second press", { tag: ["@tenants.members-list", "@tenants.invite-by-login", "@tenants.remove-member", "@tenants.refusal-in-words"] }, async ({ page }) => {
  await logIn(page);
  await account(page, "open-members");
  const view = page.getByTestId("members");
  await expect(view.getByRole("heading")).toHaveText(/^Members of /);
  await expect(view.getByTestId("member-member")).toContainText("(you)");

  // A login GitHub does not know: said in words, and what was typed stays.
  const login = view.getByLabel("GitHub login");
  await login.fill("nobody-by-this-name");
  await view.getByRole("button", { name: "Invite" }).click();
  await expect(view.getByRole("alert")).toHaveText("GitHub has no account with that login.");
  await expect(login).toHaveValue("nobody-by-this-name");

  // The invite binds to the account GitHub names, and the notice says which.
  await login.fill("Newcomer");
  await view.getByRole("button", { name: "Invite" }).click();
  await expect(page.getByRole("status")).toHaveText(/^newcomer \(GitHub user 1004\) was invited to .* as member$/);
  const row = view.getByTestId("member-newcomer");
  await expect(row).toContainText("invited");
  await expect(login).toHaveValue("");

  // Twice is refused; the last admin stays.
  await login.fill("newcomer");
  await view.getByRole("button", { name: "Invite" }).click();
  await expect(view.getByRole("alert")).toHaveText("That person is already a member.");
  await view.getByRole("button", { name: "Remove member" }).click();
  await view.getByTestId("confirm-remove-member").click();
  await expect(view.getByRole("alert")).toHaveText("A tenant keeps at least one admin. Make someone else an admin first.");

  // Removing takes a second press, and the list follows.
  await row.getByRole("button", { name: "Remove newcomer" }).click();
  await expect(row).toBeVisible();
  await view.getByTestId("confirm-remove-newcomer").click();
  await expect(view.getByTestId("member-newcomer")).toHaveCount(0);
  const listed = (await (await page.request.get("/api/t/default/members")).json()) as { login: string }[];
  expect(listed.map((m) => m.login)).toEqual(["member"]);
});

test("the operator sees tenants and the App's installations as names and counts", { tag: ["@admin.tenants-and-installations", "@admin.only-operators"] }, async ({ page }) => {
  await logIn(page);
  await account(page, "open-operator");
  const view = page.getByTestId("operator");
  const tenant = view.getByTestId("tenant-default");
  await expect(tenant).toContainText("1 member, 1 admin");
  await expect(tenant).toContainText("installation 34");
  // A tenant with a workspace and an installation is not deleted from here.
  await expect(tenant.getByRole("button", { name: "Delete" })).toBeDisabled();
  const installation = view.getByTestId("installation-34");
  await expect(installation).toContainText("m0x6A");
  await expect(installation).toContainText("attached · tenant default");
  // Detach is refused while a workspace is on the installation's repositories, in words.
  await installation.getByRole("button", { name: "Detach" }).click();
  await expect(view.getByRole("alert")).toHaveText("A workspace default of the tenant is on one of its repositories. Delete that workspace first.");
  // Nobody but an operator reaches these routes: without a session they answer 401, and the
  // unit suite covers a logged-in person who is no operator (404).
  const anonymous = await page.context().browser()!.newContext();
  expect((await anonymous.request.get(`${new URL(page.url()).origin}/api/admin/tenants`)).status()).toBe(401);
  await anonymous.close();
});
