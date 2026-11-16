// In the "room" project, after "dev" (playwright.config.ts): it pins and launches issue 919, which
// the map tests must not see.
import { expect, test } from "@playwright/test";
import WebSocket from "ws";

const base = process.env["RUBATO_E2E_URL"] ?? "http://127.0.0.1:18090";
const ISSUE = { repo: "m0x6A/rubato", number: 919 };

/** Pair a companion for `user` and keep its websocket open (ADR 0011). */
async function connectCompanion(user: string): Promise<{ ws: WebSocket; companion_id: string }> {
  const companion_id = crypto.randomUUID();
  const start = await (await fetch(`${base}/api/pair/start`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ companion_id, hostname: `${user}-agent-box`, companion_version: "e2e" }) })).json();
  const approve = await fetch(`${base}/api/pair/approve`, { method: "POST", headers: { "content-type": "application/json", "x-rubato-dev-user": user }, body: JSON.stringify({ user_code: start.user_code }) });
  expect(approve.status).toBe(200);
  const poll = await (await fetch(`${base}/api/pair/poll`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ device_code: start.device_code }) })).json();
  expect(poll.status).toBe("approved");
  const ws = new WebSocket(`${base.replace(/^http/, "ws")}/ws/companion`, { headers: { authorization: `Bearer ${poll.token}` } });
  await new Promise<void>((ok, fail) => {
    ws.once("open", () => ok());
    ws.once("error", fail);
  });
  const hello = { type: "hello", companion_id, user, companion_version: "e2e", protocol_version: 1, last_seen_server_seq: 0, capabilities: ["agent_mcp"] };
  ws.send(JSON.stringify({ seq: 1, sent_at: new Date().toISOString(), message: hello }));
  await new Promise<void>((ok) => ws.once("message", () => ok()));
  return { ws, companion_id };
}

/**
 * ADR 0071, plan 226 §3: what a launched agent says it is doing reaches the team as a line under
 * its session, with its age, through the companion's own launch.
 */
test("a launched agent's line shows under its session in the companion lane", { tag: ["@companions.agent-line"] }, async ({ page }) => {
  const pinned = await fetch(`${base}/api/pins`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ issue: ISSUE, target: { kind: "node", id: "web" } }) });
  expect(pinned.ok).toBe(true);
  const { ws, companion_id } = await connectCompanion("m0x6A");
  const launched = new Promise<{ id: string; branch: string; session_id: string }>((ok) =>
    ws.on("message", (d) => {
      const env = JSON.parse(String(d));
      if (env.message?.type === "launch") {
        ok(env.message as { id: string; branch: string; session_id: string });
      }
    }),
  );
  const r = await fetch(`${base}/api/launch`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ issue: ISSUE, slug: "agent-line" }) });
  expect(r.status, await r.clone().text()).toBe(200);
  const cmd = await launched;
  const now = () => new Date().toISOString();
  const session = { companion_id, user: "m0x6A", cwd: "/w/919", branch: cmd.branch, repo: ISSUE.repo, issue: ISSUE, status: "working", source: "hook", last_seen: now(), launch_id: cmd.id, session_id: cmd.session_id };
  ws.send(JSON.stringify({ seq: 2, sent_at: now(), message: { type: "sessions", sessions: [session] } }));
  ws.send(JSON.stringify({ seq: 3, sent_at: now(), message: { type: "agent_report", launch_id: cmd.id, doing: "wiring the agent line", parts: ["web"], at: now() } }));

  await page.goto("/");
  await page.getByRole("button", { name: /^Companions/ }).click();
  const line = page.getByTestId(`lane-${companion_id}`).getByTestId("agent-line");
  await expect(line).toContainText('"wiring the agent line"', { timeout: 20_000 });
  await expect(line).toContainText("web");
  ws.close();
});
