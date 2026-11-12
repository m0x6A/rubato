import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { listAdrs, parseAdr } from "../scripts/adrs.ts";
import { HOOK_EVENTS, renderHooks, templatePath } from "../scripts/hooks.ts";
import { render, summarize } from "../scripts/map.ts";
import { check, init } from "../scripts/plan.ts";
import { parseWorkspace, repoForCwd, workspaceCandidates, type Workspace } from "../scripts/lib/workspace.ts";

const root = resolve(import.meta.dirname, "../../..");

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8" }).trim();
}

/** A temp "connected repo" with origin set and an issue branch checked out. */
function tempRepo(origin: string, branch: string): string {
  const dir = mkdtempSync(join(tmpdir(), "rubato-skill-"));
  git(dir, "init", "-q", "-b", "main");
  git(dir, "remote", "add", "origin", `https://github.com/${origin}.git`);
  writeFileSync(join(dir, "README.md"), "x\n");
  git(dir, "-c", "user.name=t", "-c", "user.email=t@t", "add", "-A");
  git(dir, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init");
  git(dir, "checkout", "-q", "-b", branch);
  return dir;
}

/** A temp map repo with a map that has one repo override, plus two ADRs. */
function tempMapRepo(): { dir: string; ws: Workspace } {
  const dir = mkdtempSync(join(tmpdir(), "rubato-map-"));
  mkdirSync(join(dir, "docs/decisions"), { recursive: true });
  const map = readFileSync(join(root, "docs/design/rubato-map.yml"), "utf8").replace(
    "repos:\n  - name: m0x6A/rubato\n    plan_path: docs/plans/{issue}.md\n",
    "repos:\n  - name: m0x6A/rubato\n    plan_path: docs/plans/{issue}.md\n  - name: acme/pantry\n    plan_path: features/{issue}-{slug}/plan.md\n",
  );
  writeFileSync(join(dir, "system-map.yml"), map);
  writeFileSync(join(dir, "docs/decisions/0001-a.md"), "# 0001. Do a thing\n\n- **Status:** accepted\n- **Seam:** none\n\n## Decision\n\nWe do a thing.\n\n## Alternatives considered\n\n- x\n");
  writeFileSync(join(dir, "docs/decisions/0002-b.md"), "# 0002. Try b\n\n- **Status:** proposed\n- **Seam:** companion-server\n\n## Decision\n\nMaybe b.\n");
  git(dir, "init", "-q", "-b", "main");
  git(dir, "-c", "user.name=t", "-c", "user.email=t@t", "add", "-A");
  git(dir, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "map");
  const ws: Workspace = { map: { repo: "acme/map", path: "system-map.yml", local: dir }, repos: [] };
  return { dir, ws };
}

describe("workspace", () => {
  it("looks in RUBATO_WORKSPACE, then XDG, then ~/.config", () => {
    const c = workspaceCandidates({ RUBATO_WORKSPACE: "/x/w.yml", XDG_CONFIG_HOME: "/xdg" });
    expect(c[0]).toBe("/x/w.yml");
    expect(c[1]).toBe("/xdg/rubato/workspace.yml");
    expect(c[2]).toMatch(/\.config\/rubato\/workspace\.yml$/);
  });

  it("parses the example and rejects a workspace without a map", () => {
    const ws = parseWorkspace(readFileSync(join(root, "skills/rubato/workspace.example.yml"), "utf8"), "example");
    expect(ws.map.repo).toBe("m0x6A/rubato");
    expect(ws.otel?.protocol).toBe("http/protobuf");
    expect(() => parseWorkspace("repos: []\n", "bad")).toThrow(/map needs/);
    expect(repoForCwd(ws, "/home/me/develop/rubato/crates")?.name).toBe("m0x6A/rubato");
    expect(repoForCwd(ws, "/home/me/develop/other")).toBeUndefined();
  });
});

describe("map.ts", () => {
  it("summarises a valid map with its version and the repo's plan path", async () => {
    const { dir, ws } = tempMapRepo();
    const repo = tempRepo("acme/pantry", "issue/7-restock");
    const s = await summarize(ws, repo);
    expect(s.nodes.length).toBe(11);
    expect(s.seams.length).toBe(10);
    expect(s.map_version).toBe(git(dir, "rev-parse", "--short", "HEAD"));
    expect(s.repo).toBe("acme/pantry");
    expect(s.plan_path_template).toBe("features/{issue}-{slug}/plan.md");
    expect(render(s)).toContain("- rubato-companion [context]");
  });

  it("map.ts lists flows per seam", async () => {
    const { dir, ws } = tempMapRepo();
    writeFileSync(join(dir, "system-map.yml"), readFileSync(join(root, "schema/fixtures/maps/flows.yml"), "utf8"));
    const s = await summarize(ws, dir);
    const seam = s.seams.find((x) => x.id === "web-server");
    expect(seam?.flows).toEqual([
      { kind: "data", from: "server", to: "web", carries: "Overview" },
      { kind: "data", from: "web", to: "server", carries: "PinRequest" },
      { kind: "control", from: "server", to: "web", carries: "WebEvent Changed" },
    ]);
    expect(seam?.description).toBe("The browser reads the whole picture and writes pins.");
    // A seam without flows lists none.
    expect(s.seams.find((x) => x.id === "server-store")?.flows).toBeUndefined();
    const text = render(s);
    expect(text).toContain("  - data: server → web: Overview");
    expect(text).toContain("  - control: server → web: WebEvent Changed");
  });

  it("map.ts lists scenarios with their steps, which are not ids a plan may name", async () => {
    const { dir, ws } = tempMapRepo();
    writeFileSync(join(dir, "system-map.yml"), readFileSync(join(root, "schema/fixtures/maps/scenarios.yml"), "utf8"));
    const s = await summarize(ws, dir);
    expect(s.scenarios.map((x) => `${x.id}:${x.steps.length}`)).toEqual(["pin-an-issue:4", "read-back:1"]);
    const text = render(s);
    expect(text).toContain("## Scenarios (journeys over the flows above; not ids a plan may name)");
    expect(text).toContain("- pin-an-issue: Pin an issue — A person drops an issue on a part and every browser sees it.");
    expect(text).toContain("  3. control: server → web over web-server: WebEvent Changed — every browser is told something changed");
    // A map without scenarios prints no section.
    writeFileSync(join(dir, "system-map.yml"), readFileSync(join(root, "schema/fixtures/maps/flows.yml"), "utf8"));
    expect(render(await summarize(ws, dir))).not.toContain("## Scenarios");
  });

  it("refuses a map that fails the schema", async () => {
    const { dir, ws } = tempMapRepo();
    writeFileSync(join(dir, "system-map.yml"), "version: 1\nname: x\nnodes:\n  - id: a\n    kind: nonsense\n");
    await expect(summarize(ws, dir)).rejects.toThrow(/schema/);
  });
});

describe("adrs.ts", () => {
  it("lists ADRs with status and decision text", () => {
    const { ws } = tempMapRepo();
    const adrs = listAdrs(ws);
    expect(adrs.map((a) => `${a.number}:${a.status}`)).toEqual(["0001:accepted", "0002:proposed"]);
    expect(adrs[0]?.decision).toBe("We do a thing.");
    expect(parseAdr("x.md", "no title here")).toBeNull();
  });
});

describe("plan.ts", () => {
  it("writes the plan at the default path and at a per-repo override, and the result passes the check once filled", async () => {
    const { ws } = tempMapRepo();
    const a = tempRepo("m0x6A/rubato", "issue/12-thing");
    const pathA = await init(ws, a, { issue: "m0x6A/rubato#12", slug: "thing", title: "Thing" });
    expect(pathA).toBe("docs/plans/12.md");
    const b = tempRepo("acme/pantry", "issue/7-restock");
    const pathB = await init(ws, b, { issue: "acme/pantry#7", slug: "restock" });
    expect(pathB).toBe("features/7-restock/plan.md");

    // The skeleton has an empty row; a filled plan passes.
    const file = join(a, pathA);
    const filled = readFileSync(file, "utf8")
      .replace("| | affected | | |", "| rubato-companion | affected | forwards a field | companion-server |")
      .replace("| | | | | |", "| companion-server | server drops it | counter `rubato.events.dropped` | Companions | rubato-server |");
    writeFileSync(file, filled);
    const r = await check(ws, a); // path derived from the issue branch
    expect(r.ok, JSON.stringify(r.line)).toBe(true);
    expect(r.line["rubato.issue"]).toBe("m0x6A/rubato#12");

    // The skeleton's Flows table is empty and reads as no flows; a filled row is checked.
    expect(readFileSync(file, "utf8")).toContain("## Flows\n");
    expect(r.line["flows_checked"]).toBe(0);
    const flowRow = "| From | To | Seam | Kind | Carries |\n| --- | --- | --- | --- | --- |\n";
    writeFileSync(file, filled.replace(`${flowRow}| | | | | |`, `${flowRow}| rubato-companion | rubato-server | companion-server | data | the new field |`));
    const withFlow = await check(ws, a);
    expect(withFlow.ok, JSON.stringify(withFlow.line)).toBe(true);
    expect(withFlow.line["flows_checked"]).toBe(1);
    // A flow over a seam that does not join its two parts fails the check.
    writeFileSync(file, filled.replace(`${flowRow}| | | | | |`, `${flowRow}| rubato-companion | web | companion-server | data | the new field |`));
    const offSeam = await check(ws, a);
    expect(offSeam.ok).toBe(false);
    expect(offSeam.line["flows_off_seam"]).toEqual(["rubato-companion → web over companion-server"]);
    writeFileSync(file, filled);

    // And the on-purpose failure: an unknown node.
    writeFileSync(file, filled.replace("| rubato-companion |", "| rubato-orchestrator |"));
    const bad = await check(ws, a, pathA);
    expect(bad.ok).toBe(false);
    expect(bad.line["rubato.plan.unknown_ids"]).toEqual(["rubato-orchestrator"]);
  });

  it("refuses to overwrite an existing plan", async () => {
    const { ws } = tempMapRepo();
    const a = tempRepo("m0x6A/rubato", "issue/13-x");
    await init(ws, a, { issue: "m0x6A/rubato#13", slug: "x" });
    await expect(init(ws, a, { issue: "m0x6A/rubato#13", slug: "x" })).rejects.toThrow(/already exists/);
  });
});

describe("hooks.ts", () => {
  it("renders every forwarded event with port and token, as valid JSON", () => {
    const template = readFileSync(templatePath(), "utf8");
    const out = renderHooks(template, 43125, "abcdefghijklmnopqrstuvwxyz012345");
    const parsed = JSON.parse(out) as { hooks: Record<string, unknown> };
    expect(Object.keys(parsed.hooks).sort()).toEqual([...HOOK_EVENTS].sort());
    expect(out).not.toContain("{port}");
    expect(out).not.toContain("{token}");
    expect(out).toContain("http://127.0.0.1:43125/hook");
  });

  it("rejects a bad port or a short token", () => {
    const template = readFileSync(templatePath(), "utf8");
    expect(() => renderHooks(template, 0, "abcdefghijklmnopqrstuvwxyz")).toThrow(/port/);
    expect(() => renderHooks(template, 8080, "short")).toThrow(/token/);
  });
});
