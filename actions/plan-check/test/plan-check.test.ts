import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { checkPlanFile, logLine } from "../src/check.ts";
import { parseArgs } from "../src/cli.ts";
import { loadMap, loadMapText, MapError, mapIds } from "../src/map.ts";
import { parsePlan } from "../src/parse-plan.ts";

const root = resolve(import.meta.dirname, "../../..");
const schemaDir = resolve(root, "schema");
const fixtures = resolve(root, "schema/fixtures/plans");
const map = loadMap(resolve(root, "docs/design/rubato-map.yml"), schemaDir);

describe("map", () => {
  it("loads Rubato's own map", () => {
    expect(map.nodes.length).toBe(11);
    expect(map.seams?.length).toBe(10);
  });

  it("rejects a map that fails the schema or the structural checks", () => {
    const text = readFileSync(resolve(root, "schema/fixtures/bad-map.yml"), "utf8");
    let err: unknown;
    try {
      loadMapText(text, schemaDir);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(MapError);
    const problems = (err as MapError).problems.join("\n");
    expect(problems).toContain('duplicate id "a"');
    expect(problems).toContain('unknown node "b"');
    expect(problems).toContain('owner "zzz"');
  });

  it("map fixtures", () => {
    // The same two files `rubato_protocol::map::tests::map_fixtures` reads: same result on both sides.
    const dir = resolve(root, "schema/fixtures/maps");
    const good = loadMapText(readFileSync(resolve(dir, "flows.yml"), "utf8"), schemaDir);
    expect(good).toEqual(JSON.parse(readFileSync(resolve(dir, "flows.json"), "utf8")));
    let err: unknown;
    try {
      loadMapText(readFileSync(resolve(dir, "flows-off-seam.yml"), "utf8"), schemaDir);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(MapError);
    expect((err as MapError).problems).toEqual([
      'seam "web-server" has a flow from "server" to "store", which are not its two nodes',
      'seam "web-server" has a flow from "web" to "web", which are not its two nodes',
    ]);
  });

  it("scenario fixtures", () => {
    // The same files `rubato_protocol::map::tests::scenario_fixtures` reads: same problems, same order.
    const dir = resolve(root, "schema/fixtures/maps");
    const good = loadMapText(readFileSync(resolve(dir, "scenarios.yml"), "utf8"), schemaDir);
    expect(good).toEqual(JSON.parse(readFileSync(resolve(dir, "scenarios.json"), "utf8")));
    expect(mapIds(good).has("pin-an-issue")).toBe(false);
    let err: unknown;
    try {
      loadMapText(readFileSync(resolve(dir, "scenarios-bad.yml"), "utf8"), schemaDir);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(MapError);
    expect((err as MapError).problems).toEqual([
      'duplicate id "web"',
      'scenario "off-flow" step 1 on seam "web-server": no declared control flow from "web" to "server"',
      'scenario "ambiguous" step 1 on seam "web-server": 2 declared control flows from "server" to "web"; name one with carries',
      'scenario "ambiguous" step 2 on seam "web-server": no declared control flow from "server" to "web" carrying "Nope"',
      'scenario "lost" step 1 names unknown seam "web-store"',
      'scenario "empty" has no steps',
    ]);
  });

  it("an old map and an old plan validate against the new schemas", () => {
    // A map written before flows and seam descriptions existed.
    const old = loadMapText("version: 1\nname: old\nnodes:\n  - {id: a, kind: context}\n  - {id: b, kind: context}\nseams:\n  - {id: a-b, from: a, to: b, owner: a}\n", schemaDir);
    expect(old.seams?.[0]?.flows).toBeUndefined();
    // A flow of an unknown kind is refused by the schema.
    expect(() => loadMapText("version: 1\nname: x\nnodes:\n  - {id: a, kind: context}\n  - {id: b, kind: context}\nseams:\n  - {id: a-b, from: a, to: b, owner: a, flows: [{kind: magic, from: a, to: b, carries: x}]}\n", schemaDir)).toThrow(MapError);
    // The plan fixture from before the Flows table still passes.
    expect(checkPlanFile(resolve(fixtures, "valid.md"), map, schemaDir).ok).toBe(true);
    // The team's own map declares flows on every seam.
    expect((map.seams ?? []).filter((s) => !s.flows?.length || !s.description).map((s) => s.id)).toEqual([]);
  });

  it("rejects a map with a wrong kind", () => {
    expect(() => loadMapText("version: 1\nname: x\nnodes:\n  - id: a\n    kind: thing\n", schemaDir)).toThrow(MapError);
  });
});

describe("parsePlan", () => {
  it("parses the valid fixture to exactly the expected JSON (shared with the Rust parser)", () => {
    const path = "schema/fixtures/plans/valid.md";
    const plan = parsePlan(readFileSync(resolve(root, path), "utf8"), path);
    const expected = JSON.parse(readFileSync(resolve(fixtures, "valid.json"), "utf8"));
    expect(plan).toEqual(expected);
  });

  it("flows fixture matches Rust", () => {
    // `plans::parse::tests::flows_fixture_matches_typescript` reads the same two files.
    const path = "schema/fixtures/plans/flows.md";
    const plan = parsePlan(readFileSync(resolve(root, path), "utf8"), path);
    expect(plan).toEqual(JSON.parse(readFileSync(resolve(fixtures, "flows.json"), "utf8")));
    expect(plan.flows).toHaveLength(2);
    expect(() => parsePlan(readFileSync(resolve(root, path), "utf8").replace("| data |", "| magic |"), path)).toThrow(/kind must be data or control, got "magic"/);
  });

  it("parses the v0 plan's numbered headings and backticked ids", () => {
    const path = "docs/plans/v0.md";
    const plan = parsePlan(readFileSync(resolve(root, path), "utf8"), path);
    expect(plan.affected_parts.length).toBe(11);
    expect(plan.o_rows?.length).toBe(10);
    expect(plan.assumed?.length).toBeGreaterThan(10);
    expect(plan.assumed?.every((a) => a.scope === "node" || a.scope === "seam")).toBe(true);
  });

  it("rejects a plan without an issue", () => {
    expect(() => parsePlan("---\ntitle: x\n---\n## Affected parts\n| Node | Status | What changes |\n| --- | --- | --- |\n", "p.md")).toThrow(/issue/);
  });
});

describe("check", () => {
  it("passes the valid fixture and reports what it checked", () => {
    const r = checkPlanFile(resolve(fixtures, "valid.md"), map, schemaDir);
    expect(r.ok).toBe(true);
    expect(r.idsChecked).toBe(5);
    const line = logLine(r);
    expect(line["event"]).toBe("rubato.plan.check");
    expect(line["rubato.issue"]).toBe("m0x6A/rubato#42");
    expect(line["rubato.plan.unknown_ids"]).toEqual([]);
  });

  it("counts legacy front-matter keys and still passes (ADR 0027)", () => {
    const r = checkPlanFile(resolve(fixtures, "legacy-keys.md"), map, schemaDir);
    expect(r.ok).toBe(true);
    expect(r.legacyKeys).toEqual(["review", "approval"]);
    expect(logLine(r)["legacy_keys"]).toBe(2);
    expect(logLine(r)["legacy_key_names"]).toEqual(["review", "approval"]);
    const clean = checkPlanFile(resolve(fixtures, "valid.md"), map, schemaDir);
    expect(clean.legacyKeys).toEqual([]);
    expect(logLine(clean)["legacy_keys"]).toBe(0);
  });

  it("a flow off its seam fails", () => {
    const r = checkPlanFile(resolve(fixtures, "flow-off-seam.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.flowsOffSeam).toEqual(["web → github over web-server"]);
    expect(r.unknownIds).toEqual([]);
    expect(r.flowsChecked).toBe(2);
    expect(logLine(r)["flows_off_seam"]).toEqual(["web → github over web-server"]);
  });

  it("an unknown flow id fails", () => {
    const r = checkPlanFile(resolve(fixtures, "unknown-flow-id.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.unknownIds).toEqual(["rubato-orchestrator", "web-nowhere"]);
    // An unknown seam is an unknown id, not also a flow off its seam.
    expect(r.flowsOffSeam).toEqual([]);
  });

  it("a plan without Flows passes", () => {
    const old = checkPlanFile(resolve(fixtures, "valid.md"), map, schemaDir);
    expect(old.ok).toBe(true);
    expect(old.plan?.flows).toBeUndefined();
    expect(logLine(old)["flows_checked"]).toBe(0);
    const withFlows = checkPlanFile(resolve(fixtures, "flows.md"), map, schemaDir);
    expect(withFlows.ok).toBe(true);
    expect(logLine(withFlows)["flows_checked"]).toBe(2);
    expect(logLine(withFlows)["flows_off_seam"]).toEqual([]);
  });

  it("fails on purpose: unknown node", () => {
    const r = checkPlanFile(resolve(fixtures, "unknown-node.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.unknownIds).toEqual(["rubato-orchestrator"]);
  });

  it("fails on purpose: seam without an O-row", () => {
    const r = checkPlanFile(resolve(fixtures, "missing-o-row.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.seamsWithoutORow).toEqual(["server-github"]);
    expect(logLine(r)["rubato.plan.o_rows_missing"]).toEqual(["server-github"]);
  });

  it("fails on purpose: status outside the schema", () => {
    const r = checkPlanFile(resolve(fixtures, "bad-status.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.schemaErrors.join(" ")).toMatch(/status|enum/);
  });

  it("passes every plan in this repo (dogfood)", () => {
    for (const f of ["docs/plans/v0.md", "docs/plans/1.md"]) {
      const r = checkPlanFile(resolve(root, f), map, schemaDir);
      expect(r, JSON.stringify(logLine(r))).toMatchObject({ ok: true });
    }
  });
});

describe("cli args", () => {
  it("defaults plans to docs/plans/*.md and requires --map", () => {
    expect(parseArgs(["--map", "m.yml"]).plans).toEqual(["docs/plans/*.md"]);
    expect(() => parseArgs([])).toThrow(/--map/);
  });
});
