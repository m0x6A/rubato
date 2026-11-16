import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parse as parseYaml } from "yaml";
import type { SystemMap } from "../protocol.ts";
import { isRetired, liveMap } from "./map.ts";

const root = resolve(import.meta.dirname, "../../..");
const real = parseYaml(readFileSync(resolve(root, "docs/design/rubato-map.yml"), "utf8")) as SystemMap;

describe("liveMap (ADR 0077)", () => {
  it("drops retired nodes and seams, and keeps everything else as it is", () => {
    const live = liveMap(real);
    expect(live.nodes.map((n) => n.id)).not.toContain("tmux-ide");
    expect((live.seams ?? []).map((s) => s.id)).not.toContain("companion-tmux-ide");
    expect(live.nodes.length).toBe(real.nodes.length - 1);
    expect((live.seams ?? []).length).toBe((real.seams ?? []).length - 1);
    expect(live.scenarios).toBe(real.scenarios);
    expect(isRetired(real, "tmux-ide")).toBe(true);
    expect(isRetired(real, "companion-tmux-ide")).toBe(true);
    expect(isRetired(real, "rubato-companion")).toBe(false);
    expect(isRetired(real, "no-such-id")).toBe(false);
  });

  it("is the map itself when nothing is retired", () => {
    const plain: SystemMap = { version: 1, name: "x", nodes: [{ id: "a", kind: "context" }], seams: [] };
    expect(liveMap(plain)).toEqual(plain);
  });
});
