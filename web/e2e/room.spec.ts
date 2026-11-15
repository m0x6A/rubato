import { type Page, expect, test } from "@playwright/test";
import WebSocket from "ws";

const base = process.env["RUBATO_E2E_URL"] ?? "http://127.0.0.1:18090";
const BRANCH = "issue/916-room";
const ISSUE = { repo: "m0x6A/rubato", number: 916 };

/** Pair a companion for `user` that can take room turns, and keep its websocket open. */
async function connectCompanion(user: string): Promise<{ ws: WebSocket; companion_id: string }> {
  const companion_id = crypto.randomUUID();
  const start = await (await fetch(`${base}/api/pair/start`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ companion_id, hostname: `${user}-room-box`, companion_version: "e2e" }) })).json();
  const approve = await fetch(`${base}/api/pair/approve`, { method: "POST", headers: { "content-type": "application/json", "x-rubato-dev-user": user }, body: JSON.stringify({ user_code: start.user_code }) });
  expect(approve.status).toBe(200);
  const poll = await (await fetch(`${base}/api/pair/poll`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ device_code: start.device_code }) })).json();
  expect(poll.status).toBe("approved");
  const ws = new WebSocket(`${base.replace(/^http/, "ws")}/ws/companion`, { headers: { authorization: `Bearer ${poll.token}` } });
  await new Promise<void>((ok, fail) => {
    ws.once("open", () => ok());
    ws.once("error", fail);
  });
  const hello = { type: "hello", companion_id, user, companion_version: "e2e", protocol_version: 1, last_seen_server_seq: 0, capabilities: ["session_input", "room_turns"] };
  ws.send(JSON.stringify({ seq: 1, sent_at: new Date().toISOString(), message: hello }));
  await new Promise<void>((ok) => ws.once("message", () => ok()));
  return { ws, companion_id };
}

async function openRoom(page: Page) {
  await page.goto("/");
  await page.getByTestId("issue-916").click();
  await expect(page.getByTestId("room")).toBeVisible();
}

async function post(page: Page, text: string) {
  await page.getByLabel("Message to the room").fill(text);
  await page.getByRole("button", { name: "Post", exact: true }).click();
  await expect(page.getByRole("list", { name: "Messages in the room" })).toContainText(text);
}

/**
 * Issue #122 (ADR 0057), with two people: what one posts in a plan's room the other reads, and a
 * teammate's request to call the owner's agent in waits until the owner accepts it.
 */
test("two people talk in a plan's room, and a teammate asks the owner to call their agent in", async ({ page, browser }) => {
  // The issue is pinned; the poll reads its plan on issue/916-room, so it has a room.
  const pinned = await fetch(`${base}/api/pins`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ issue: ISSUE, target: { kind: "node", id: "skill-rubato" } }) });
  expect(pinned.ok).toBe(true);
  await expect
    .poll(async () => {
      const o = (await (await fetch(`${base}/api/overview`)).json()) as { issues: { issue: { number: number }; plan?: { git_ref?: string } }[] };
      return o.issues.find((i) => i.issue.number === 916)?.plan?.git_ref;
    }, { timeout: 30_000 })
    .toBe(BRANCH);

  const alice = await (await browser.newContext({ extraHTTPHeaders: { "x-rubato-dev-user": "alice" } })).newPage();
  await openRoom(page);
  await openRoom(alice);
  await post(page, "From m0x6A: the table stays.");
  await post(alice, "From alice: agreed, one question.");
  await page.reload();
  await page.getByTestId("issue-916").click();
  const log = page.getByRole("list", { name: "Messages in the room" });
  await expect(log).toContainText("From m0x6A: the table stays.");
  await expect(log).toContainText("From alice: agreed, one question.");

  // m0x6A launches a session on the plan's branch through a companion that takes room turns.
  const { ws, companion_id } = await connectCompanion("m0x6A");
  const launched = new Promise<{ id: string; branch: string; session_id: string }>((ok) =>
    ws.on("message", (d) => {
      const env = JSON.parse(String(d));
      if (env.message?.type === "launch") {
        ok(env.message as { id: string; branch: string; session_id: string });
      }
    }),
  );
  const r = await fetch(`${base}/api/launch`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ issue: ISSUE, slug: "room" }) });
  expect(r.status, await r.clone().text()).toBe(200);
  const cmd = await launched;
  expect(cmd.branch).toBe(BRANCH);
  const session = { companion_id, user: "m0x6A", cwd: "/w/916", branch: BRANCH, repo: ISSUE.repo, issue: ISSUE, status: "idle", source: "hook", last_seen: new Date().toISOString(), launch_id: cmd.id, session_id: cmd.session_id };
  ws.send(JSON.stringify({ seq: 2, sent_at: new Date().toISOString(), message: { type: "sessions", sessions: [session] } }));

  // alice asks; the request is pending, and the owner sees it with Accept and Decline.
  await expect.poll(async () => {
    await alice.reload();
    await alice.getByTestId("issue-916").click();
    return alice.getByRole("button", { name: "Ask m0x6A to call their agent in" }).count();
  }, { timeout: 20_000 }).toBe(1);
  await alice.getByRole("button", { name: "Ask m0x6A to call their agent in" }).click();
  await alice.getByRole("button", { name: "Ask", exact: true }).click();
  await expect(alice.getByText("Asked m0x6A; the request is pending.")).toBeVisible();
  await page.reload();
  await page.getByTestId("issue-916").click();
  const requests = page.getByRole("list", { name: "Requests to call an agent in" });
  await expect(requests).toContainText("alice asks you to call your agent in");
  await requests.getByRole("button", { name: "Accept" }).click();
  await expect(page.getByText("Accepted: your agent joins the room.")).toBeVisible();
  await expect(page.getByRole("list", { name: "In the room", exact: true })).toContainText("m0x6A-agent");
  ws.close();
});
