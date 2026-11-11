import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { parse as parseYaml } from "yaml";
import WebSocket from "ws";

const root = resolve(import.meta.dirname, "../..");
const map = parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")) as { nodes: { id: string }[] };
const base = process.env["RUBATO_E2E_URL"] ?? "http://127.0.0.1:18090";

/** Pair a companion for `user` through the API and keep its websocket open. The companion side of ADR 0011. */
async function connectCompanion(user: string): Promise<WebSocket> {
  const companion_id = crypto.randomUUID();
  const start = await (await fetch(`${base}/api/pair/start`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ companion_id, hostname: `${user}-box`, companion_version: "e2e" }) })).json();
  const approve = await fetch(`${base}/api/pair/approve`, { method: "POST", headers: { "content-type": "application/json", "x-rubato-dev-user": user }, body: JSON.stringify({ user_code: start.user_code }) });
  expect(approve.status).toBe(200);
  const poll = await (await fetch(`${base}/api/pair/poll`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ device_code: start.device_code }) })).json();
  expect(poll.status).toBe("approved");
  const ws = new WebSocket(`${base.replace(/^http/, "ws")}/ws/companion`, { headers: { authorization: `Bearer ${poll.token}` } });
  await new Promise<void>((ok, fail) => {
    ws.once("open", () => ok());
    ws.once("error", fail);
  });
  ws.send(JSON.stringify({ seq: 1, sent_at: new Date().toISOString(), message: { type: "hello", companion_id, user, companion_version: "e2e", protocol_version: 1, last_seen_server_seq: 0 } }));
  await new Promise<void>((ok) => ws.once("message", () => ok()));
  return ws;
}

test("the map renders every node id from system-map.yml", async ({ page }) => {
  await page.goto("/");
  for (const n of map.nodes) {
    await expect(page.getByTestId(`node-${n.id}`)).toBeVisible();
  }
  await expect(page.getByText("map rubato @")).toBeVisible();
});

test("drag a not-pinned issue onto a part and onto a seam", async ({ page }) => {
  // The harness serves three open issues of m0x6A/rubato: #901, #902 and #903, none tracked.
  const key = (n: number) => `m0x6A/rubato#${n}`;
  await page.goto("/");
  const backlog = page.getByTestId("backlog");
  await expect(backlog.getByTestId("backlog-row-902")).toContainText("Backlog: an unpinned issue");
  await expect(backlog.getByTestId("backlog-row-903")).toContainText("Backlog: another unpinned issue");
  await expect(page.getByTestId("issue-row-902")).toHaveCount(0);
  await expect(page.getByTestId("issue-row-903")).toHaveCount(0);

  // Onto a part. A pin selects the issue, so its detail opens: Planned, pinned to the part.
  await backlog.getByTestId("backlog-row-902").dragTo(page.getByTestId("node-rubato-server"));
  const detail = page.getByTestId("issue-detail");
  await expect(detail).toContainText(key(902));
  await expect(detail).toContainText("Planned");
  await expect(detail).toContainText("node rubato-server");
  await page.getByRole("button", { name: "← all issues" }).click();
  await expect(page.getByTestId("issue-row-902")).toContainText("Planned");
  await expect(page.getByTestId("issue-row-902")).toContainText("rubato-server");
  await expect(page.getByTestId("backlog-row-902")).toHaveCount(0);

  // Onto a seam: the drop target is the seam's chip.
  await page.getByTestId("backlog-row-903").dragTo(page.getByTestId("seam-web-server"));
  await expect(detail).toContainText(key(903));
  await expect(detail).toContainText("Planned");
  await expect(detail).toContainText("seam web-server");
  await page.getByRole("button", { name: "← all issues" }).click();
  await expect(page.getByTestId("issue-row-903")).toContainText("Planned");
  await expect(page.getByTestId("issue-row-903")).toContainText("web-server");
  await expect(page.getByTestId("backlog-row-903")).toHaveCount(0);

  // The issue nobody dragged is still not pinned, and both targets now count their issue.
  await expect(page.getByTestId("backlog-row-901")).toBeVisible();
  await expect(page.getByTestId("node-rubato-server")).toContainText("planned");
  await expect(page.getByTestId("seam-web-server")).toContainText("1 issue");
});

test("start session appears only for the current user's own connected companion", async ({ page }) => {
  // Pin an issue so there is something to select.
  const pin = await fetch(`${base}/api/pins`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ issue: { repo: "m0x6A/rubato", number: 9 }, target: { kind: "node", id: "rubato-server" } }) });
  expect([201, 200]).toContain(pin.status);

  await page.goto("/");
  await page.getByTestId("issue-9").click();
  await expect(page.getByTestId("issue-detail")).toBeVisible();
  await expect(page.getByTestId("no-companion")).toBeVisible();
  await expect(page.getByTestId("start-session")).toHaveCount(0);

  // Alice's companion connects: still no button for m0x6A.
  const alice = await connectCompanion("alice");
  await page.reload();
  await page.getByTestId("issue-9").click();
  await expect(page.getByTestId("no-companion")).toBeVisible();
  await expect(page.getByTestId("start-session")).toHaveCount(0);
  await page.getByRole("button", { name: /Companions/ }).click();
  await expect(page.getByTestId("companion-alice")).toContainText("connected");

  // m0x6A's own companion connects: the button appears, and a launch reaches that socket.
  const mine = await connectCompanion("m0x6A");
  const launched = new Promise<string>((ok) => mine.on("message", (d) => {
    const env = JSON.parse(String(d));
    if (env.message?.type === "launch") {
      ok(env.message.branch as string);
    }
  }));
  await page.reload();
  await page.getByTestId("issue-9").click();
  await expect(page.getByTestId("start-session")).toBeVisible();
  await page.getByTestId("start-session").click();
  expect(await launched).toMatch(/^issue\/9-/);
  await expect(page.getByRole("status")).toContainText("sent to your companion");
  alice.close();
  mine.close();
});
