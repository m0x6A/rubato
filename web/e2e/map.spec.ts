import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { type Page, expect, test } from "@playwright/test";
import { parse as parseYaml } from "yaml";
import WebSocket from "ws";
import { ghostTraffic, routeOverview } from "./fixtures/traffic.ts";

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

test("By repo boxes each repo's parts, and the Repos tab says what each repo provides (issue #89)", async ({ page }) => {
  const nodes = (parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")) as { nodes: { id: string; repo?: string }[] }).nodes;
  const owned = nodes.filter((n) => n.repo);
  await page.goto("/");
  await expect(page.getByTestId(`node-${owned[0]!.id}`)).toBeVisible();
  await page.getByRole("button", { name: "By repo" }).click();
  const box = page.getByTestId("repo-box-m0x6A/rubato");
  await expect(box).toBeVisible();
  // Every part with a repo is drawn inside its box; the externals outside it.
  const b = (await box.boundingBox())!;
  for (const n of nodes) {
    const p = (await page.getByTestId(`node-${n.id}`).boundingBox())!;
    const inside = p.x >= b.x && p.y >= b.y && p.x + p.width <= b.x + b.width + 1 && p.y + p.height <= b.y + b.height + 1;
    expect(inside, n.id).toBe(Boolean(n.repo));
  }
  await page.getByRole("button", { name: "By repo" }).click();
  await expect(box).toHaveCount(0);

  await page.getByTestId("tab-repos").click();
  const provides = page.getByTestId("provides-m0x6A/rubato");
  await expect(provides).toContainText(`${owned.length} parts`);
  await provides.locator("summary").click();
  await expect(provides).toContainText(owned[0]!.id);
});

test("a member's Map: line places an issue without a drag; a stranger's only suggests", async ({ page }) => {
  // fixtures/issues.json: #912 by the owner names seam companion-server; #913 by someone
  // outside the team names part plan-check (ADR 0026).
  await page.goto("/");
  await expect(page.getByTestId("issue-row-912")).toContainText("Planned");
  await expect(page.getByTestId("issue-row-912")).toContainText("companion-server");
  await expect(page.getByTestId("backlog-row-912")).toHaveCount(0);
  await expect(page.getByTestId("seam-companion-server")).toContainText("1 issue");

  const stranger = page.getByTestId("backlog-row-913");
  await expect(stranger).toBeVisible();
  await expect(page.getByTestId("issue-row-913")).toHaveCount(0);
  const suggestion = page.getByTestId("suggestion-913");
  await expect(suggestion).toContainText("Suggested: part plan-check");
  await expect(suggestion).toContainText("from someone outside the team");
});

test("an issue created from a part is pinned on it (ADR 0025)", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("node-plan-check").click();
  await page.getByTestId("inspector").getByTestId("new-issue-here").click();
  const form = page.getByTestId("new-issue");
  await form.getByLabel("Title").fill("Plan check: report the flows it checked");
  await form.getByLabel("Summary").fill("The plan check counts flows but does not name them.");
  await form.getByLabel("What the change does").fill("Names each flow it checked in the log line");
  await form.getByLabel("How we'll know it works").fill("The fixture run prints the flow names");
  await form.getByLabel("What it must not break").fill("The count in the summary line");
  await form.getByRole("button", { name: "Create in m0x6A/rubato" }).click();
  // The new issue opens, Planned, pinned to the part it was started on.
  const detail = page.getByTestId("issue-detail");
  // The title arrives with the next listing (the poll runs every 5 s here).
  await expect(detail).toContainText("Plan check: report the flows it checked", { timeout: 15_000 });
  await expect(detail).toContainText("Planned");
  await expect(detail).toContainText("node plan-check");
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

/** Where a part is drawn on the canvas, from React Flow's own transform (not the screen, which depends on pan and zoom). */
async function canvasPosition(page: Page, id: string): Promise<{ x: number; y: number }> {
  const transform = await page.locator(`.react-flow__node[data-id="${id}"]`).evaluate((n) => (n as HTMLElement).style.transform);
  const m = /translate\((-?[\d.]+)px,\s*(-?[\d.]+)px\)/.exec(transform);
  if (!m) {
    throw new Error(`no position for ${id}: ${transform}`);
  }
  return { x: Math.round(Number(m[1])), y: Math.round(Number(m[2])) };
}

test("a dragged part is in the same place in a second browser and after reload", async ({ page, browser }) => {
  const other = await (await browser.newContext({ baseURL: base })).newPage();
  await page.goto("/");
  await other.goto("/");
  const part = page.getByTestId("node-web");
  await expect(part).toBeVisible();
  await expect(other.getByTestId("node-web")).toBeVisible();
  const before = await canvasPosition(page, "web");
  expect(await canvasPosition(other, "web")).toEqual(before);
  const neighbour = await canvasPosition(page, "rubato-server");

  // Drag with the mouse, in steps, as a person does.
  const box = (await part.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2 + 45, { steps: 6 });
  await page.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2 + 90, { steps: 6 });
  await page.mouse.up();

  const after = await canvasPosition(page, "web");
  expect(after).not.toEqual(before);
  // The second browser follows over the websocket, without a reload; only the dragged part moved.
  await expect.poll(() => canvasPosition(other, "web")).toEqual(after);
  expect(await canvasPosition(other, "rubato-server")).toEqual(neighbour);
  // The seams still hang on the moved part.
  await expect(other.getByTestId("seam-web-server")).toBeVisible();

  // Still there after a reload, in both.
  await page.reload();
  await expect(page.getByTestId("node-web")).toBeVisible();
  await expect.poll(() => canvasPosition(page, "web")).toEqual(after);

  // The keyboard path, as React Flow announces it to a screen reader: focus the part, Enter
  // to select it, then the arrow keys move it. Saved the same way.
  await page.locator('.react-flow__node[data-id="web"]').focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => canvasPosition(page, "web").then((p) => p.x)).toBeGreaterThan(after.x);
  const nudged = await canvasPosition(page, "web");
  await expect.poll(() => canvasPosition(other, "web")).toEqual(nudged);

  // Auto-arrange removes every saved position for everyone; Undo brings them back.
  await page.getByRole("button", { name: "Auto-arrange" }).click();
  await expect(page.getByRole("status")).toContainText("Auto-arranged for everyone: 1 saved position removed");
  await expect.poll(() => canvasPosition(other, "web")).toEqual(before);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toContainText("The saved positions are back");
  await expect.poll(() => canvasPosition(other, "web")).toEqual(nudged);
  await expect.poll(() => canvasPosition(page, "web")).toEqual(nudged);

  // Leave the shared layout as it was found, for the tests that follow.
  await page.getByRole("button", { name: "Auto-arrange" }).click();
  await expect.poll(() => canvasPosition(page, "web")).toEqual(before);
  await other.context().close();
});

test("the data flow lens shows rubato-map.yml's flows with arrowheads", async ({ page }) => {
  const seams = (parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")) as { seams: { id: string; flows?: { kind: string; carries: string }[] }[] }).seams;
  const flows = (kind: string) => seams.flatMap((s) => (s.flows ?? []).filter((f) => f.kind === kind).map((f) => ({ seam: s.id, ...f })));
  const withHead = page.locator(".react-flow__edge-path[marker-end], .react-flow__edge-path[marker-start]");
  await page.goto("/");
  await expect(page.getByTestId("seam-web-server")).toBeVisible();

  // Structure: one arrow per seam, every one with a head, and no flow labels.
  await expect(page.getByRole("radio", { name: "Structure" })).toBeChecked();
  await expect(withHead).toHaveCount(seams.length);
  await expect(page.locator('[data-testid^="flow-"]')).toHaveCount(0);

  for (const kind of ["data", "control"] as const) {
    const declared = flows(kind);
    const seamsWithout = seams.filter((s) => !(s.flows ?? []).some((f) => f.kind === kind)).length;
    await page.getByRole("radio", { name: new RegExp(`^${kind === "data" ? "Data" : "Control"} flow \\(${declared.length}\\)$`) }).click();
    // One labelled arrow per declared flow of the kind, and a faint plain arrow per seam without one.
    await expect(page.locator('[data-testid^="flow-"]')).toHaveCount(declared.length);
    await expect(withHead).toHaveCount(declared.length + seamsWithout);
    await expect(page.locator(".react-flow__edge.faint")).toHaveCount(seamsWithout);
    await expect(page.locator(`.react-flow__edge.flow.${kind}`)).toHaveCount(declared.length);
    // Every seam keeps exactly one chip, so it stays selectable and droppable in every lens.
    await expect(page.locator('[data-testid^="seam-"]')).toHaveCount(seams.length);
    const first = declared[0]!;
    await expect(page.locator(`[data-testid^="flow-${first.seam}#${kind}-0"]`)).toHaveAttribute("title", `${kind} flow over ${first.seam}: ${first.carries}`);
    await expect(page.getByTestId("legend")).toContainText(kind === "data" ? "solid arrow" : "dashed arrow");
  }
  // Control arrows are dashed.
  await expect(page.locator(".react-flow__edge.flow.control .react-flow__edge-path").first()).toHaveAttribute("style", /stroke-dasharray/);

  // Selecting a part explains it and dims what is not next to it.
  await page.getByRole("radio", { name: "Structure" }).click();
  await page.getByTestId("node-rubato-companion").click();
  const inspector = page.getByTestId("inspector");
  await expect(inspector.getByRole("heading", { name: "rubato-companion" })).toBeVisible();
  await expect(inspector).toContainText("One per developer machine.");
  await expect(inspector).toContainText("LaunchCommand, for the paired user only from rubato-server over companion-server");
  await expect(page.getByTestId("node-rubato-server")).not.toHaveClass(/dim/);
  await expect(page.getByTestId("node-web")).toHaveClass(/dim/);
  // Walk the map from the inspector: to the seam, which explains itself.
  await inspector.getByRole("button", { name: "companion-server" }).first().click();
  await expect(inspector.getByRole("heading", { name: "companion-server" })).toBeVisible();
  await expect(inspector).toContainText("rubato-server is woken when it breaks");
  await inspector.getByRole("button", { name: "Close" }).click();
  await expect(inspector).toHaveCount(0);
  await expect(page.getByTestId("node-web")).not.toHaveClass(/dim/);
});

/** Each part's card lies inside the map canvas and does not touch the band of controls above it. */
async function expectClear(page: Page, parts: string[]): Promise<void> {
  await expect(async () => {
    const band = (await page.getByTestId("map-band").boundingBox())!;
    const canvas = (await page.locator(".map-canvas").boundingBox())!;
    for (const id of parts) {
      const box = (await page.getByTestId(`node-${id}`).boundingBox())!;
      const overlaps = box.x < band.x + band.width && box.x + box.width > band.x && box.y < band.y + band.height && box.y + box.height > band.y;
      expect(overlaps, `${id} is under the controls`).toBe(false);
      expect(box.x >= canvas.x && box.y >= canvas.y && box.x + box.width <= canvas.x + canvas.width && box.y + box.height <= canvas.y + canvas.height, `${id} is inside the canvas`).toBe(true);
    }
  }).toPass({ timeout: 3000 });
}

for (const motion of ["no-preference", "reduce"] as const) {
  test(`play "start a session on an issue" step by step (motion: ${motion})`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: motion });
    await page.goto("/");
    await expect(page.getByTestId("seam-web-server")).toBeVisible();
    await page.getByRole("radio", { name: "Scenarios (4)" }).click();
    const player = page.getByRole("group", { name: "Scenario player" });
    await expect(player.getByRole("combobox", { name: "Scenario" })).toHaveValue("start-a-session");
    const caption = page.getByTestId("scenario-caption");
    const pulse = page.locator('[data-testid^="pulse-"]');
    const active = page.locator(".react-flow__edge.step-active");

    // Step 1: web tells the server to start. The control pulse is a ring on that one arrow.
    await expect(caption).toHaveText(/^Step 1 of 11: web → rubato-server, control: LaunchRequest/);
    await expect(active).toHaveCount(1);
    await expect(pulse).toHaveCount(1);
    await expect(pulse).toHaveClass(/control/);
    await expect(page.getByTestId("node-web")).not.toHaveClass(/dim/);
    await expect(page.getByTestId("node-rubato-server")).not.toHaveClass(/dim/);
    await expect(page.getByTestId("node-rubato-companion")).toHaveClass(/dim/);
    // Moving (a CSS animation on the pulse), or still under reduced motion (issue #81).
    expect(await pulse.evaluate((e) => e.getAnimations().length)).toBe(motion === "reduce" ? 0 : 1);
    await expect(page.locator("animateMotion")).toHaveCount(0);
    await expectClear(page, ["web", "rubato-server"]);

    // Step 2: the launch goes to the companion; the lit parts follow the step.
    await player.getByRole("button", { name: "Next step" }).click();
    await expect(caption).toHaveText(/^Step 2 of 11: rubato-server → rubato-companion, control: LaunchCommand/);
    await expect(page.getByTestId("node-rubato-companion")).not.toHaveClass(/dim/);
    await expect(page.getByTestId("node-web")).toHaveClass(/dim/);
    await expect(active).toHaveCount(1);

    // Step 5 is data coming back: a filled dot, not a ring.
    for (let i = 0; i < 3; i++) {
      await player.getByRole("button", { name: "Next step" }).click();
    }
    await expect(caption).toHaveText(/^Step 5 of 11: rubato-companion → rubato-server, data:/);
    await expect(pulse).toHaveClass(/data/);

    // Nothing covers the step: its two parts are inside the canvas and clear of the band of
    // controls above it, at every step, even zoomed in so far that the map has to pan.
    const steps = parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")).scenarios[0].steps as { from: string; to: string }[];
    const zoomIn = page.locator(".react-flow__controls-zoomin");
    for (let i = 0; i < 4; i++) {
      await zoomIn.click();
    }
    await player.getByRole("button", { name: "Previous step" }).click();
    for (let i = 3; i < steps.length; i++) {
      await expect(caption).toHaveText(new RegExp(`^Step ${i + 1} of 11:`));
      await expectClear(page, [steps[i]!.from, steps[i]!.to]);
      if (i < steps.length - 1) {
        await player.getByRole("button", { name: "Next step" }).click();
      }
    }

    // Back to the structure lens: no pulse, nothing dimmed.
    await page.getByRole("radio", { name: "Structure" }).click();
    await expect(pulse).toHaveCount(0);
    await expect(page.getByTestId("node-web")).not.toHaveClass(/dim/);
  });
}

test("selecting an issue with a plan paints its parts and flows", async ({ page }) => {
  // The seeded repository has branch issue/901-lens with a plan that has a Flows table. Pinning
  // #901 makes it a tracked issue; the next poll tick (5 s in this harness) reads the plan.
  const pin = await page.request.post("/api/pins", { data: { issue: { repo: "m0x6A/rubato", number: 901 }, target: { kind: "node", id: "rubato-server" } } });
  expect(pin.status()).toBe(201);
  await page.goto("/");
  const row = page.getByTestId("issue-row-901");
  await expect(row).toContainText("Designed", { timeout: 20_000 });

  // The team view first: no plan lens, the switch works.
  await expect(page.getByRole("radio", { name: "Structure" })).toBeEnabled();
  await expect(page.locator(".react-flow__edge.moving")).toHaveCount(0);

  await row.click();
  await expect(page.getByTestId("issue-detail")).toContainText("m0x6A/rubato#901");
  // The plan's parts are painted, every other part steps back.
  await expect(page.getByTestId("node-rubato-server")).toHaveClass(/paint-affected/);
  await expect(page.getByTestId("node-web")).toHaveClass(/paint-affected/);
  await expect(page.getByTestId("node-rubato-server")).not.toHaveClass(/dim/);
  for (const outside of ["rubato-companion", "github", "plan-file", "tmux-ide"]) {
    await expect(page.getByTestId(`node-${outside}`)).toHaveClass(/dim/);
  }
  // Its two flows move along web-server, one in each direction; no other seam carries one.
  await expect(page.locator(".react-flow__edge.moving")).toHaveCount(2);
  await expect(page.getByTestId("flow-web-server#plan-0")).toHaveAttribute("title", "data flow over web-server: the new overview field");
  await expect(page.getByTestId("flow-web-server#plan-1")).toHaveAttribute("title", "control flow over web-server: the request that asks for it");
  await expect(page.locator(".react-flow__edge.moving .react-flow__edge-path[marker-start]")).toHaveCount(1);
  await expect(page.locator(".react-flow__edge.moving .react-flow__edge-path[marker-end]")).toHaveCount(1);
  await expect(page.getByTestId("legend")).toContainText("The plan of m0x6A/rubato#901.");
  await expect(page.getByRole("radio", { name: "Structure" })).toBeDisabled();
  // The panel says what the plan says.
  const plan = page.getByTestId("plan-inspector");
  await expect(plan).toContainText("rubato-server affected serves one more field in the overview");
  await expect(plan).toContainText("data the new overview field: rubato-server → web over web-server");
  await expect(plan).toContainText("web-server (the field is missing in the overview)");

  // Clearing the selection returns to the team view.
  await page.getByRole("button", { name: "← all issues" }).click();
  await expect(page.locator(".react-flow__edge.moving")).toHaveCount(0);
  await expect(page.getByTestId("node-github")).not.toHaveClass(/dim/);
  await expect(page.getByRole("radio", { name: "Structure" })).toBeEnabled();
  await expect(plan).toHaveCount(0);
});

test("queue an issue on a paired companion and start it", async ({ page }) => {
  // Issue #9 was pinned by an earlier test; pin it here too so the test stands alone.
  await page.request.post("/api/pins", { data: { issue: { repo: "m0x6A/rubato", number: 9 }, target: { kind: "node", id: "rubato-server" } } });
  const alice = await connectCompanion("alice");
  const mine = await connectCompanion("m0x6A");
  const commands: { id: string; branch: string }[] = [];
  mine.on("message", (d) => {
    const env = JSON.parse(String(d));
    if (env.message?.type === "launch") {
      commands.push({ id: env.message.id as string, branch: env.message.branch as string });
    }
  });
  await page.goto("/");
  const row = page.getByTestId("issue-row-9");
  await expect(row).toBeVisible();

  // Drag the issue over the Companions tab, which opens, and drop it on my own lane.
  const tab = page.getByRole("button", { name: /^Companions/ });
  await row.hover();
  await page.mouse.down();
  await tab.hover();
  await tab.hover();
  const myLane = page.locator('[data-testid^="lane-"][aria-label^="Your companion m0x6A-box"]').last();
  await expect(myLane).toBeVisible();
  await myLane.hover();
  await myLane.hover();
  await page.mouse.up();
  await expect(page.getByRole("status")).toContainText("m0x6A/rubato#9 queued on m0x6A-box");
  const entry = myLane.getByTestId("queued-9");
  await expect(entry).toContainText("queued by m0x6A");

  // Alice's lane is there for everyone to see, with no controls for me, and takes no drop.
  const aliceLane = page.locator('[data-testid^="lane-"][aria-label^="alice\'s companion"]').last();
  await expect(aliceLane).toContainText("Nothing queued.");
  const refused = await page.request.post("/api/queue", { data: { issue: { repo: "m0x6A/rubato", number: 9 }, companion_id: (await aliceLane.getAttribute("data-testid"))!.replace("lane-", "") } });
  expect(refused.status()).toBe(403);

  // Start: the launch reaches my companion's socket; the entry stays until the companion answers.
  await entry.getByRole("button", { name: "Start m0x6A/rubato#9" }).click();
  await expect.poll(() => commands.length).toBe(1);
  expect(commands[0]!.branch).toMatch(/^issue\/9-/);
  await expect(page.getByRole("status")).toContainText("It leaves the queue when the companion reports it started.");
  await expect(entry).toBeVisible();

  // The companion refuses: the entry stays and says why.
  const answer = (seq: number, message: Record<string, unknown>) => mine.send(JSON.stringify({ seq, sent_at: new Date().toISOString(), message: { type: "launch", command_id: commands.at(-1)!.id, at: new Date().toISOString(), ...message } }));
  answer(2, { outcome: "refused", reason: "missing_dependency", detail: "tmux-ide 2.9 is needed" });
  await expect(entry).toContainText("did not start: missing_dependency: tmux-ide 2.9 is needed");

  // Started the second time: the entry leaves the lane.
  await entry.getByRole("button", { name: "Start m0x6A/rubato#9" }).click();
  await expect.poll(() => commands.length).toBe(2);
  answer(3, { outcome: "started", session_id: crypto.randomUUID(), worktree: "/w", tmux_session: "s" });
  await expect(myLane.getByTestId("queued-9")).toHaveCount(0);
  await expect(myLane).toContainText("Nothing queued.");
  alice.close();
  mine.close();
});

test("draft a part and a seam, propose", async ({ page }) => {
  const repo = process.env["RUBATO_E2E_REPO"];
  test.skip(!repo, "needs the seeded repository of e2e/run.sh");
  const git = (...args: string[]) => execFileSync("git", ["-C", repo!, ...args], { encoding: "utf8" }).trim();
  const draftBranches = () => git("for-each-ref", "--format=%(refname:short)", "refs/heads/map/").split("\n").filter(Boolean);
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  expect(draftBranches()).toEqual([]);

  // Edit mode: the draft panel opens. Add a part with the form.
  await page.getByRole("button", { name: "Edit map" }).click();
  const panel = page.getByTestId("draft-panel");
  await expect(panel).toContainText("empty");
  const addPart = panel.getByRole("form", { name: "Add part" });
  await addPart.getByLabel(/^Id/).fill("billing");
  await addPart.getByLabel("What it is").fill("Invoices and payments.");
  await addPart.getByRole("button", { name: "Add part" }).click();
  // The part is on the map for everyone, dashed and badged with who drafted it.
  const billing = page.getByTestId("node-billing");
  await expect(billing).toBeVisible();
  await expect(billing).toHaveClass(/draft-added/);
  await expect(billing).toContainText("draft · m0x6A");
  await expect(panel).toContainText("add part billing");

  // Draw a seam: drag from the right handle of web to the left handle of billing.
  const from = page.locator('.react-flow__node[data-id="web"] .react-flow__handle.source');
  const to = page.locator('.react-flow__node[data-id="billing"] .react-flow__handle.target');
  await from.hover();
  await page.mouse.down();
  const target = (await to.boundingBox())!;
  await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 12 });
  await page.mouse.up();
  const seam = page.getByTestId("seam-web-billing");
  await expect(seam).toContainText("draft · m0x6A");
  await expect(panel).toContainText("add seam web-billing (web to billing)");

  // A draft id is not a map id: it cannot be pinned, and an invalid change says why.
  const pin = await page.request.post("/api/pins", { data: { issue: { repo: "m0x6A/rubato", number: 903 }, target: { kind: "node", id: "billing" } } });
  expect(pin.status()).toBe(422);
  await addPart.getByLabel(/^Id/).fill("web");
  await addPart.getByRole("button", { name: "Add part" }).click();
  await expect(panel.getByRole("alert")).toContainText("Part web was not added: id web already exists");

  // Give the seam a flow through its form, then look at what the proposal changes.
  await seam.click();
  const seamForm = panel.getByRole("form", { name: "Change seam web-billing" });
  await seamForm.getByLabel(/^Flows/).fill("data: billing -> web: cost per issue");
  await seamForm.getByRole("button", { name: "Save seam" }).click();
  await expect(panel).toContainText("change seam web-billing");
  await panel.getByRole("button", { name: "Propose map change…" }).click();
  const diff = page.getByTestId("draft-diff");
  await expect(diff).toContainText("+ - id: billing");
  await expect(diff).toContainText("carries: cost per issue");
  expect(draftBranches()).toEqual([]);

  // Confirm: one branch in the real repository, started at main, holding the new map file.
  await page.getByRole("button", { name: "Open the pull request" }).click();
  await expect(page.getByRole("status")).toContainText("Map change proposed as pull request #1");
  await expect(panel.getByTestId("proposal")).toContainText("Proposed");
  const branches = draftBranches();
  expect(branches).toHaveLength(1);
  expect(branches[0]).toMatch(/^map\/draft-\d{8}-\d{6}$/);
  const file = parseYaml(git("show", `${branches[0]}:docs/design/rubato-map.yml`)) as { nodes: { id: string; description?: string }[]; seams: { id: string; from: string; to: string; flows?: { carries: string }[] }[] };
  expect(file.nodes.find((n) => n.id === "billing")?.description).toBe("Invoices and payments.");
  expect(file.seams.find((s) => s.id === "web-billing")).toMatchObject({ from: "web", to: "billing", flows: [{ carries: "cost per issue" }] });
  expect(file.nodes).toHaveLength(map.nodes.length + 1);
  expect(git("rev-list", "--count", `main..${branches[0]}`)).toBe("1");
  expect(git("show", "main:docs/design/rubato-map.yml")).not.toContain("billing");

  // Proposed is read-only for everyone: no forms, and a write is refused.
  await expect(panel.getByRole("form")).toHaveCount(0);
  const refused = await page.request.post("/api/map/draft/ops", { data: { op: "add_node", op_id: crypto.randomUUID(), node: { id: "ledger", kind: "context" } } });
  expect(refused.status()).toBe(409);

  // Withdraw: the branch goes, and the draft is editable again with every change kept.
  await panel.getByRole("button", { name: "Withdraw proposal" }).click();
  await expect(page.getByRole("status")).toContainText("Proposal withdrawn");
  expect(draftBranches()).toEqual([]);
  await expect(panel).toContainText("3 changes");
  await expect(panel.getByRole("form", { name: "Add part" })).toBeVisible();

  // Discard empties it for everyone; Undo brings it back; discard again to leave the map as found.
  await panel.getByRole("button", { name: "Discard draft" }).click();
  await expect(page.getByTestId("node-billing")).toHaveCount(0);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(page.getByTestId("node-billing")).toBeVisible();
  await expect(panel).toContainText("3 changes");
  await panel.getByRole("button", { name: "Discard draft" }).click();
  await expect(page.getByTestId("node-billing")).toHaveCount(0);
  await expect(panel).toContainText("empty");
});

test("a ghost taken into the draft becomes a part with its service, and its call a seam (issue #140)", async ({ page }) => {
  // The seeded server has no traffic source: a report with two ghosts is added on the way to the page.
  await routeOverview(page, ghostTraffic);
  await page.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  // Where the part sits on the canvas, not on the screen: the band above may grow.
  const placed = () => page.locator('.react-flow__node[data-id="rubato-server"]').evaluate((el) => (el as HTMLElement).style.transform);
  const before = await placed();
  await page.getByRole("radio", { name: "Traffic" }).click();
  await expect(page.getByTestId("ghost-shop/checkout")).toBeVisible();
  // The declared parts stay where they were: the ghosts sit in their own band.
  expect(await placed()).toBe(before);
  const list = page.getByTestId("discrepancies");
  // Closed by default, so the map keeps its room; the count says there is something to see.
  await expect(list.locator(":scope > summary")).toContainText("Map versus territory (6)");
  await list.locator(":scope > summary").click();
  await expect(list).toContainText("shop/checkout is running but not on the map");
  // Its call waits for its part.
  const edgeRow = list.getByTestId("discrepancy-edge").filter({ hasText: "shop/checkout calls rubato-server" });
  await expect(edgeRow.getByRole("button", { name: "take the part into the draft first" })).toBeDisabled();

  // Take the ghost: the draft opens with the form filled in, and nothing is sent until it is submitted.
  const ghostRow = list.getByTestId("discrepancy-ghost").filter({ hasText: "shop/checkout" });
  await ghostRow.getByRole("button", { name: "Add as a part…" }).click();
  const panel = page.getByTestId("draft-panel");
  await expect(panel).toContainText("empty");
  const addPart = panel.getByRole("form", { name: "Add part" });
  await expect(addPart.getByLabel(/^Id/)).toHaveValue("checkout");
  await expect(addPart.getByLabel("service.namespace", { exact: true })).toHaveValue("shop");
  await expect(addPart.getByLabel("service.name", { exact: true })).toHaveValue("checkout");
  await addPart.getByLabel("What it is").fill("The shop's checkout.");
  await addPart.getByRole("button", { name: "Add part" }).click();
  await expect(panel).toContainText("add part checkout");

  // Now the call can follow, from the drafted part to the server.
  await page.getByRole("button", { name: "Edit map" }).click();
  await page.getByRole("radio", { name: "Traffic" }).click();
  if (!(await list.evaluate((el) => (el as HTMLDetailsElement).open))) {
    await list.locator(":scope > summary").click();
  }
  await expect(list.getByTestId("discrepancy-ghost").filter({ hasText: "shop/checkout" })).toContainText("in the draft as checkout");
  await edgeRow.getByRole("button", { name: "Add as a seam…" }).click();
  const addSeam = panel.getByRole("form", { name: "Add seam" });
  await expect(addSeam.getByLabel(/^From/)).toHaveValue("checkout");
  await expect(addSeam.getByLabel(/^To/)).toHaveValue("rubato-server");
  await addSeam.getByRole("button", { name: "Add seam" }).click();
  await expect(panel).toContainText("add seam checkout-rubato-server");

  // The proposed map file keeps the service exactly as it was emitted.
  const proposed = parseYaml(await (await page.request.get("/api/map/draft/yaml")).text()) as { nodes: { id: string; service?: { namespace?: string; name: string } }[] };
  expect(proposed.nodes.find((n) => n.id === "checkout")?.service).toEqual({ namespace: "shop", name: "checkout" });

  // Leave the map as found.
  await panel.getByRole("button", { name: "Discard draft" }).click();
  await expect(panel).toContainText("empty");
});

// Issue #144: the By repo grouping, readable and arranged by the team (ADR 0068). These run last
// in this file, after the flat layout's own test, and leave every arrangement as they found it.
const DEV = { "x-rubato-dev-user": "m0x6A" };
const BOX = "repo-box-m0x6A/rubato";
const PAD = { left: 24, top: 48 };

async function byRepo(page: Page): Promise<void> {
  await expect(page.getByTestId("node-web")).toBeVisible();
  await page.getByRole("button", { name: "By repo" }).click();
  await expect(page.getByTestId(BOX)).toBeVisible();
}

/** A node's top-left on the canvas; for a part in a box, relative to its box. */
async function inBox(page: Page, id: string, box = "repo:m0x6A/rubato"): Promise<{ x: number; y: number }> {
  const p = await canvasPosition(page, id);
  const b = await canvasPosition(page, box);
  return { x: p.x - b.x, y: p.y - b.y };
}

async function dragBy(page: Page, target: ReturnType<Page["getByTestId"]>, dx: number, dy: number): Promise<void> {
  const r = (await target.boundingBox())!;
  const at = { x: r.x + Math.min(40, r.width / 2), y: r.y + Math.min(14, r.height / 2) };
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.mouse.move(at.x + dx / 2, at.y + dy / 2, { steps: 6 });
  await page.mouse.move(at.x + dx, at.y + dy, { steps: 6 });
  await page.mouse.up();
}

async function partIds(page: Page): Promise<string[]> {
  return page.locator(".react-flow__node-map").evaluateAll((ns) => ns.map((n) => (n as HTMLElement).dataset["id"] ?? ""));
}

async function flatPositions(page: Page): Promise<Record<string, { x: number; y: number }>> {
  const out: Record<string, { x: number; y: number }> = {};
  for (const id of await partIds(page)) {
    out[id] = await canvasPosition(page, id);
  }
  return out;
}

async function clearGrouped(page: Page, ws = ""): Promise<void> {
  const r = await page.request.delete(`${base}${ws ? `/api/w/${ws}` : "/api"}/layout/grouped`, { headers: DEV });
  expect(r.status()).toBe(200);
}

for (const lens of ["Structure", "Data flow", "Control flow"]) {
  test(`By repo on the team's map, ${lens} lens: no chip, label or badge overlaps another or a part card, and every name is readable`, async ({ page }) => {
    await page.goto("/");
    await byRepo(page);
    await page.getByRole("radio", { name: new RegExp(`^${lens}`) }).click();
    await page.waitForTimeout(300);
    const boxes = await page.evaluate(() => {
      const rect = (e: Element, what: string) => {
        const r = e.getBoundingClientRect();
        return { what, x: r.left, y: r.top, w: r.width, h: r.height };
      };
      return [
        ...[...document.querySelectorAll(".seam-label")].map((e) => rect(e, (e.textContent ?? "").slice(0, 40))),
        ...[...document.querySelectorAll(".react-flow__node-map")].map((e) => rect(e, `part ${(e as HTMLElement).dataset["id"]}`)),
      ];
    });
    const clash: string[] = [];
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i]!;
        const b = boxes[j]!;
        if (a.what.startsWith("part ") && b.what.startsWith("part ")) {
          continue;
        }
        if (a.x < b.x + b.w - 1 && b.x < a.x + a.w - 1 && a.y < b.y + b.h - 1 && b.y < a.y + a.h - 1) {
          clash.push(`${a.what} / ${b.what}`);
        }
      }
    }
    console.log(`${lens}: checked ${boxes.length} labels and cards`);
    expect(clash).toEqual([]);
    // Every part's name is whole: not clipped by its card.
    const clipped = await page.locator(".react-flow__node-map .id").evaluateAll((ns) => ns.filter((n) => n.scrollWidth > n.clientWidth + 1).map((n) => n.textContent));
    expect(clipped).toEqual([]);
  });
}

test("By repo: a part dragged inside its box stays there for everyone and after a reload; By repo off brings back the flat map exactly", async ({ page, browser }) => {
  const other = await (await browser.newContext({ baseURL: base })).newPage();
  await page.goto("/");
  await other.goto("/");
  await expect(page.getByTestId("node-web")).toBeVisible();
  const flat = await flatPositions(page);
  await byRepo(page);
  await byRepo(other);
  const before = await inBox(page, "web");

  await dragBy(page, page.getByTestId("node-web"), 40, 160);
  const after = await inBox(page, "web");
  expect(after).not.toEqual(before);
  // Inside its box.
  const box = (await page.getByTestId(BOX).boundingBox())!;
  const part = (await page.getByTestId("node-web").boundingBox())!;
  expect(part.x >= box.x && part.y >= box.y && part.x + part.width <= box.x + box.width + 1 && part.y + part.height <= box.y + box.height + 1).toBe(true);
  // The second browser follows over the websocket, box size included.
  await expect.poll(() => inBox(other, "web")).toEqual(after);
  await expect.poll(async () => (await other.locator(`.react-flow__node[data-id="repo:m0x6A/rubato"]`).evaluate((n) => (n as HTMLElement).style.height))).toBe(await page.locator(`.react-flow__node[data-id="repo:m0x6A/rubato"]`).evaluate((n) => (n as HTMLElement).style.height));

  // Still there after a reload, with By repo on again.
  await page.reload();
  await byRepo(page);
  await expect.poll(() => inBox(page, "web")).toEqual(after);

  // By repo off: every part is where the flat map has it, untouched by the grouping.
  await page.getByRole("button", { name: "By repo" }).click();
  await expect(page.getByTestId(BOX)).toHaveCount(0);
  await expect.poll(() => flatPositions(page)).toEqual(flat);
  const stored = await (await page.request.get(`${base}/api/overview`, { headers: DEV })).json();
  expect(Object.keys(stored.layout.positions ?? {})).toEqual([]);
  expect(stored.layout.grouped.web).toBeDefined();

  await clearGrouped(page);
  await other.context().close();
});

test("By repo: a part dragged past its box's edge stays inside; a box moves with its parts", async ({ page }) => {
  await page.goto("/");
  await byRepo(page);
  // Past the left edge (and a little down): it stops at the box's padding.
  const was = await inBox(page, "web");
  await dragBy(page, page.getByTestId("node-web"), -400, 30);
  await expect.poll(() => inBox(page, "web").then((p) => p.y)).toBeGreaterThan(was.y);
  await expect.poll(() => inBox(page, "web").then((p) => p.x)).toBe(PAD.left);

  // The box, by its name bar: every part in it moves by the same offset, the externals do not.
  const ids = await partIds(page);
  const inside = new Map<string, { x: number; y: number }>();
  for (const id of ids) {
    inside.set(id, await canvasPosition(page, id));
  }
  const boxAt = await canvasPosition(page, "repo:m0x6A/rubato");
  await dragBy(page, page.locator(".repo-box-frame").first(), 0, 120);
  await expect.poll(() => canvasPosition(page, "repo:m0x6A/rubato").then((p) => p.y)).toBeGreaterThan(boxAt.y);
  const moved = await canvasPosition(page, "repo:m0x6A/rubato");
  const d = { x: moved.x - boxAt.x, y: moved.y - boxAt.y };
  const externals = map.nodes.filter((n) => !(n as { repo?: string }).repo).map((n) => n.id);
  for (const id of ids) {
    const was = inside.get(id)!;
    const want = externals.includes(id) ? was : { x: was.x + d.x, y: was.y + d.y };
    expect(await canvasPosition(page, id), id).toEqual(want);
  }
  // Saved for the team (the write follows the drop after a short pause), and after a reload the same.
  await expect.poll(async () => (await (await page.request.get(`${base}/api/overview`, { headers: DEV })).json()).layout.boxes?.["m0x6A/rubato"]).toBeDefined();
  await page.reload();
  await byRepo(page);
  await expect.poll(() => canvasPosition(page, "repo:m0x6A/rubato")).toEqual(moved);
  await expect.poll(() => inBox(page, "web").then((p) => p.x)).toBe(PAD.left);

  // Auto-arrange in By repo clears only the grouping, for everyone; Undo brings it back.
  await page.getByRole("button", { name: "Auto-arrange" }).click();
  await expect(page.getByRole("status")).toContainText("By repo auto-arranged for everyone: 2 saved positions removed");
  await expect.poll(() => canvasPosition(page, "repo:m0x6A/rubato")).not.toEqual(moved);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toContainText("The saved positions are back");
  await expect.poll(() => canvasPosition(page, "repo:m0x6A/rubato")).toEqual(moved);
  await clearGrouped(page);
});

test("By repo in a workspace with two repos: a part or a box dropped on the other box goes back", async ({ page }) => {
  await page.goto("/?w=shop");
  await expect(page.getByTestId("node-storefront")).toBeVisible();
  await page.getByRole("button", { name: "By repo" }).click();
  const ledger = page.getByTestId("repo-box-acme/ledger");
  await expect(ledger).toBeVisible();
  const start = await canvasPosition(page, "storefront");
  const target = (await ledger.boundingBox())!;
  const from = (await page.getByTestId("node-storefront").boundingBox())!;
  await dragBy(page, page.getByTestId("node-storefront"), target.x + target.width / 2 - from.x - 40, target.y + target.height / 2 - from.y - 14);
  await expect.poll(() => canvasPosition(page, "storefront")).toEqual(start);
  // The shop box by its name bar onto the ledger box: back too.
  const shopAt = await canvasPosition(page, "repo:acme/shop");
  const shopBar = page.getByTestId("repo-box-acme/shop").locator(".repo-box-frame");
  const bar = (await shopBar.boundingBox())!;
  await dragBy(page, shopBar, target.x + 30 - bar.x, target.y + 10 - bar.y);
  await expect.poll(() => canvasPosition(page, "repo:acme/shop")).toEqual(shopAt);
  const ov = await (await page.request.get(`${base}/api/w/shop/overview`, { headers: DEV })).json();
  expect(ov.layout.grouped ?? {}).toEqual({});
  expect(ov.layout.boxes ?? {}).toEqual({});
});

test("By repo: a bundle opens on hover and on a selected part; a flow label reads in full on hover", async ({ page }) => {
  await page.goto("/");
  await byRepo(page);
  const badge = page.getByTestId("bundle-skill-rubato|system-map");
  await expect(badge).toHaveText("1 seam");
  await expect(page.getByTestId("seam-skill-system-map")).toHaveCount(0);
  // The seams that leave the repo are always drawn.
  await expect(page.getByTestId("seam-server-github")).toBeVisible();
  await badge.hover();
  await expect(page.getByTestId("seam-skill-system-map")).toBeVisible();
  await page.mouse.move(5, 5);
  await expect(page.getByTestId("seam-skill-system-map")).toHaveCount(0);
  await page.getByTestId("node-skill-rubato").click();
  await expect(page.getByTestId("seam-skill-system-map")).toBeVisible();
  await expect(page.getByTestId("seam-skill-plan-file")).toBeVisible();
  // Its warning stays on the badge: companion-server has two open plans touching it.
  await page.getByTestId("inspector").getByRole("button", { name: "Close" }).click();
  await expect(page.getByTestId("bundle-rubato-companion|rubato-server")).toContainText("⚠");

  // Data flow: a long flow label is clipped until hovered, then read whole.
  await page.getByRole("radio", { name: /^Data flow/ }).click();
  const label = page.getByTestId("flow-server-github#data-0");
  const clipped = () => label.locator(".flow-text").evaluate((e) => e.scrollWidth > e.clientWidth + 1);
  expect(await clipped()).toBe(true);
  await label.hover();
  await expect.poll(clipped).toBe(false);
});

test("By repo: issues drop on a part and on a seam inside a box, New issue here… opens, and Edit map turns the grouping off", async ({ page }) => {
  await page.goto("/");
  await byRepo(page);
  const backlog = page.getByTestId("backlog");
  await backlog.getByTestId("backlog-row-917").dragTo(page.getByTestId("node-plan-check"));
  const detail = page.getByTestId("issue-detail");
  await expect(detail).toContainText("m0x6A/rubato#917");
  await expect(detail).toContainText("node plan-check");
  await page.getByRole("button", { name: "← all issues" }).click();
  // Onto a seam in a bundle: the bundle opens under the dragged issue, and the drop lands on its chip.
  await backlog.getByTestId("backlog-row-918").dragTo(page.getByTestId("bundle-skill-rubato|system-map"));
  await expect(detail).toContainText("m0x6A/rubato#918");
  await expect(detail).toContainText("seam skill-system-map");
  await page.getByRole("button", { name: "← all issues" }).click();

  await page.getByTestId("node-web").click();
  await page.getByTestId("inspector").getByTestId("new-issue-here").click();
  await expect(page.getByTestId("new-issue")).toBeVisible();
  await page.goto("/");
  await byRepo(page);

  await page.getByRole("button", { name: "Edit map" }).click();
  await expect(page.getByTestId(BOX)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "By repo" })).toBeDisabled();
  await page.getByRole("button", { name: "Edit map" }).click();
  await expect(page.getByTestId(BOX)).toBeVisible();

  for (const [n, target] of [
    [917, { kind: "node", id: "plan-check" }],
    [918, { kind: "seam", id: "skill-system-map" }],
  ] as const) {
    const r = await page.request.delete(`${base}/api/pins`, { headers: DEV, data: { issue: { repo: "m0x6A/rubato", number: n }, target } });
    expect(r.ok()).toBe(true);
  }
});
