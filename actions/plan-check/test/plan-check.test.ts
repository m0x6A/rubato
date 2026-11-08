import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { checkPlanFile, logLine } from "../src/check.ts";
import { parseArgs } from "../src/cli.ts";
import { loadMap, loadMapText, MapError } from "../src/map.ts";
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
