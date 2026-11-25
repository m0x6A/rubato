import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { checkPlanFile, logLine } from "../src/check.ts";
import { parseArgs } from "../src/cli.ts";
import { loadMap, loadMapText, MapError, mapIds, retiredMapIds, structuralProblems, type SystemMap } from "../src/map.ts";
import { parse as parseYaml } from "yaml";
import { parsePlan } from "../src/parse-plan.ts";
import { planSha } from "../src/plan-sha.ts";
import { bundleHash, bundlePaths } from "../src/bundle.ts";

const root = resolve(import.meta.dirname, "../../..");
const schemaDir = resolve(root, "schema");
const fixtures = resolve(root, "schema/fixtures/plans");
const map = loadMap(resolve(root, "docs/design/rubato-map.yml"), schemaDir);

describe("map", () => {
  it("loads Rubato's own map", () => {
    expect(map.nodes.length).toBe(14);
    expect(map.seams?.length).toBe(15);
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

  it("retired fixtures (ADR 0077)", () => {
    // The same files `rubato_protocol::map::tests::retired_fixtures` reads: same problems, same order.
    const dir = resolve(root, "schema/fixtures/maps");
    const good = loadMapText(readFileSync(resolve(dir, "retired.yml"), "utf8"), schemaDir);
    expect(good).toEqual(JSON.parse(readFileSync(resolve(dir, "retired.json"), "utf8")));
    expect([...retiredMapIds(good)]).toEqual(["runner", "companion-runner"]);
    expect(mapIds(good).has("runner")).toBe(true);
    const badText = readFileSync(resolve(dir, "retired-bad.yml"), "utf8");
    const problems = structuralProblems(parseYaml(badText) as SystemMap);
    expect(problems).toEqual([
      'node "runner" retired "70" is not an ADR number (four digits)',
      'node "tracer" is retired but names service "tracer"; a retired part matches no traffic',
      'seam "companion-runner" is retired but declares flows',
      'seam "companion-tracer" is live but ends at retired node "tracer"',
      'seam "companion-agent" retired "ADR 70" is not an ADR number (four digits)',
    ]);
    // Loaded, the schema's pattern refuses a value that is not four digits before any rule runs.
    let err: unknown;
    try {
      loadMapText(badText, schemaDir);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(MapError);
    expect((err as MapError).problems.join("\n")).toContain("/nodes/1/retired");
    console.log(`checked 2 retired map fixtures, ${problems.length} problems`);
  });

  it("refuses node and seam ids outside the map id rule (issue #155)", () => {
    // The same file `rubato-protocol/tests/map_ids.rs` reads as its must-fail case.
    const text = readFileSync(resolve(root, "schema/fixtures/maps/bad-ids.yml"), "utf8");
    let err: unknown;
    try {
      loadMapText(text, schemaDir);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(MapError);
    const problems = (err as MapError).problems.filter((p) => p.includes("must match pattern"));
    expect(problems.map((p) => p.split(" ")[0])).toEqual(["/nodes/1/id", "/nodes/2/id", "/nodes/3/id", "/seams/1/id"]);
  });

  it("service fixtures", () => {
    // The same files `rubato_protocol::map::tests::service_fixtures` reads (ADR 0059).
    const dir = resolve(root, "schema/fixtures/maps");
    const good = loadMapText(readFileSync(resolve(dir, "services.yml"), "utf8"), schemaDir);
    expect(good).toEqual(JSON.parse(readFileSync(resolve(dir, "services.json"), "utf8")));
    let err: unknown;
    try {
      loadMapText(readFileSync(resolve(dir, "services-duplicate.yml"), "utf8"), schemaDir);
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(MapError);
    expect((err as MapError).problems).toEqual([
      'parts "checkout" and "checkout-v2" both name service "shop/checkout"; a service is one part',
      'parts "github" and "gh" both name service "github"; a service is one part',
    ]);
    // An unknown key inside service fails the schema (additionalProperties: false).
    expect(() => loadMapText("version: 1\nname: x\nnodes:\n  - id: a\n    kind: context\n    service: { name: a, namspace: typo }\n", schemaDir)).toThrow(MapError);
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
    // The team's own map declares flows on every live seam; a retired one carries nothing
    // (ADR 0077); every seam has a description.
    expect((map.seams ?? []).filter((s) => !s.description || (!s.flows?.length && s.retired === undefined)).map((s) => s.id)).toEqual([]);
    expect((map.seams ?? []).filter((s) => s.retired !== undefined && s.flows?.length).map((s) => s.id)).toEqual([]);
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

  it("data models fixture matches Rust (ADR 0031)", () => {
    // `plans::parse::tests::data_models_fixture_matches_typescript` reads the same files.
    const path = "schema/fixtures/plans/data-models.md";
    const text = readFileSync(resolve(root, path), "utf8");
    const plan = parsePlan(text, path);
    expect(plan).toEqual(JSON.parse(readFileSync(resolve(fixtures, "data-models.json"), "utf8")));
    const none = "schema/fixtures/plans/data-models-none.md";
    expect(parsePlan(readFileSync(resolve(root, none), "utf8"), none).data_models).toBeUndefined();
    expect(() => parsePlan(text.replace("| table |", "| blob |"), path)).toThrow(/kind must be table, type, schema, event or file, got "blob"/);
    expect(() => parsePlan(text.replace("| new |", "| added |"), path)).toThrow(/change must be new, changed or removed, got "added"/);
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

  it("checks data models and reports how many (ADR 0031)", () => {
    const r = checkPlanFile(resolve(fixtures, "data-models.md"), map, schemaDir);
    expect(r, JSON.stringify(logLine(r))).toMatchObject({ ok: true, dataModelsChecked: 3 });
    expect(logLine(r)["data_models"]).toBe(3);
    const none = checkPlanFile(resolve(fixtures, "data-models-none.md"), map, schemaDir);
    expect(none.ok).toBe(true);
    expect(logLine(none)["data_models"]).toBe(0);
  });

  it("fails on purpose: a data model on a part the plan does not name", () => {
    const r = checkPlanFile(resolve(fixtures, "model-off-part.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.unknownIds).toEqual([]);
    expect(r.modelsOffPart).toEqual(["user_tokens on rubato-server"]);
    expect(logLine(r)["models_off_part"]).toEqual(["user_tokens on rubato-server"]);
  });

  it("fails on purpose: a data model crossing a seam the plan does not touch", () => {
    const r = checkPlanFile(resolve(fixtures, "model-off-seam.md"), map, schemaDir);
    expect(r.ok).toBe(false);
    expect(r.modelsOffPart).toEqual([]);
    expect(r.modelsOffSeam).toEqual(["HookEvent over companion-server"]);
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

  it("reports a retired id a plan names, and does not fail it (ADR 0077)", () => {
    const r = checkPlanFile(resolve(root, "docs/plans/v0.md"), map, schemaDir);
    expect(r.ok).toBe(true);
    expect(r.retiredIds).toContain("companion-tmux-ide");
    expect(logLine(r)["rubato.plan.retired_ids"]).toEqual(r.retiredIds);
    // On purpose: the same map without `retired` reports nothing, so the count below would fail.
    const unretired: SystemMap = { ...map, nodes: map.nodes.map(({ retired: _, ...n }) => n), seams: (map.seams ?? []).map(({ retired: _, ...s }) => s) };
    expect(checkPlanFile(resolve(root, "docs/plans/v0.md"), unretired, schemaDir).retiredIds).toEqual([]);
    // Every merged plan that names tmux-ide or its seam: history checks, and is counted.
    const naming = readdirSync(resolve(root, "docs/plans"))
      .filter((f) => f.endsWith(".md"))
      .map((f) => checkPlanFile(resolve(root, "docs/plans", f), map, schemaDir))
      .filter((x) => x.retiredIds.length > 0);
    expect(naming.every((x) => x.ok)).toBe(true);
    expect(naming.length).toBe(10);
    console.log(`checked ${naming.length} plans that name retired ids`);
  });

  it("passes every plan in this repo (dogfood)", () => {
    for (const f of ["docs/plans/v0.md", "docs/plans/1.md", "docs/plans/80.md"]) {
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

describe("several maps (issue #89, ADR 0045)", () => {
  it("passes a plan that fits any map naming the repo, and says which", async () => {
    const { main } = await import("../src/cli.ts");
    const { mkdtempSync, writeFileSync } = await import("node:fs");
    const { tmpdir } = await import("node:os");
    const dir = mkdtempSync(resolve(tmpdir(), "plan-check-"));
    // A second map for the same repo with a part the team's map lacks.
    const own = readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8");
    const shop = own.replace("nodes:\n", "nodes:\n  - id: rubato-orchestrator\n    kind: context\n    repo: m0x6A/rubato\n");
    writeFileSync(resolve(dir, "shop.yml"), shop);
    const plan = readFileSync(resolve(root, "docs/plans/89.md"), "utf8").replace("| rubato-protocol | affected |", "| rubato-orchestrator | affected | orchestrates | web-server |\n| rubato-protocol | affected |");
    writeFileSync(resolve(dir, "89.md"), plan);
    const lines: string[] = [];
    const log = console.log;
    console.log = (s: string) => lines.push(s);
    try {
      expect(main(["--map", resolve(root, "docs/design/rubato-map.yml"), "--map", resolve(dir, "shop.yml"), "--repo", "m0x6A/rubato", "--plans", resolve(dir, "89.md")])).toBe(0);
      const line = JSON.parse(lines[0] as string);
      expect(line.maps_checked).toBe(2);
      expect(line.fits).toEqual([resolve(dir, "shop.yml")]);
      expect(lines[1]).toContain("and 1 other map(s)");
      // The on-purpose failure: the team's map alone does not have the part.
      lines.length = 0;
      expect(main(["--map", resolve(root, "docs/design/rubato-map.yml"), "--plans", resolve(dir, "89.md")])).toBe(1);
      expect(JSON.parse(lines[0] as string).maps_checked).toBeUndefined();
    } finally {
      console.log = log;
    }
  });

  it("takes --map more than once and --repo", () => {
    const a = parseArgs(["--map", "a.yml", "--map", "b.yml", "--repo", "acme/auth"]);
    expect(a.map).toBe("a.yml");
    expect(a.maps).toEqual(["a.yml", "b.yml"]);
    expect(a.repo).toBe("acme/auth");
  });
});

describe("plan form for approval in Rubato (ADR 0054, ADR 0056)", () => {
  it("decisions fixture matches Rust", () => {
    // `plans::parse::tests::decisions_and_the_approval_record_match_typescript` reads the same files.
    const path = "schema/fixtures/plans/decisions.md";
    const plan = parsePlan(readFileSync(resolve(root, path), "utf8"), path);
    expect(plan).toEqual(JSON.parse(readFileSync(resolve(fixtures, "decisions.json"), "utf8")));
    expect(plan.decisions).toHaveLength(2);
    expect(parsePlan(readFileSync(resolve(fixtures, "valid.md"), "utf8"), "v.md").decisions).toBeUndefined();
  });

  it("the approval hash matches the shared fixtures, and the record does not change it", () => {
    const dir = resolve(fixtures, "approval-hash");
    const cases = readdirSync(dir).filter((f) => f.endsWith(".md")).sort();
    expect(cases).toEqual(["crlf.md", "edited.md", "ticked.md", "unticked.md", "with-keys.md"]);
    for (const f of cases) {
      expect(planSha(readFileSync(resolve(dir, f), "utf8")), f).toBe(readFileSync(resolve(dir, f.replace(/\.md$/, ".txt")), "utf8").trim());
    }
    console.log(`checked ${cases.length} approval-hash fixtures`);
  });

  it("passes a current approval record and fails a stale or partial one, on purpose", () => {
    const current = checkPlanFile(resolve(fixtures, "approval-current.md"), map, schemaDir);
    expect(current.approval).toBe("current");
    expect(current.ok).toBe(true);
    const stale = checkPlanFile(resolve(fixtures, "approval-stale.md"), map, schemaDir);
    expect(stale.approval).toBe("stale");
    expect(stale.ok).toBe(false);
    const partial = checkPlanFile(resolve(fixtures, "approval-partial.md"), map, schemaDir);
    expect(partial.approval).toBe("partial");
    expect(partial.ok).toBe(false);
    const none = checkPlanFile(resolve(fixtures, "valid.md"), map, schemaDir);
    expect(none.approval).toBe("none");
    expect(none.ok).toBe(true);
    expect(logLine(stale).approval).toBe("stale");
  });

  it("bundle paths and hash match the shared fixture (Rust: plan_hash tests)", () => {
    const dir = resolve(fixtures, "bundle");
    const c = JSON.parse(readFileSync(resolve(dir, "case.json"), "utf8"));
    expect(bundlePaths(readFileSync(resolve(dir, "plan.md"), "utf8"), c.plan_path)).toEqual(c.paths);
    expect(bundleHash(c.plan_path, c.files)).toBe(c.hash);
    const edited = c.files.map(([p, t]: [string, string], i: number) => [p, i === 1 ? t + "one more line\n" : t]);
    expect(bundleHash(c.plan_path, edited)).not.toBe(c.hash);
    expect(bundlePaths("[a](../../../etc/passwd) [d](122-x/design.md#d1) [e](https://x.test/122-x/y.md)\n", "docs/plans/122.md")).toEqual(["docs/plans/122.md", "docs/plans/122-x/design.md"]);
  });
});
