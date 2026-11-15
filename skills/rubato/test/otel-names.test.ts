import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { ATTR, SPAN } from "../scripts/otel-emit.ts";

const root = resolve(import.meta.dirname, "../../..");
const otelRs = readFileSync(join(root, "crates/rubato-protocol/src/otel.rs"), "utf8");

/** Every string constant in otel.rs: the `rubato.*` names and the semconv keys. */
function known(): Set<string> {
  return new Set([...otelRs.matchAll(/pub const [A-Z_]+: &str = "([^"]+)";/g)].map((m) => m[1] as string));
}

/** The `rubato.*` names a source file writes as string literals. */
export function namesIn(text: string): string[] {
  return [...text.matchAll(/["'`](rubato\.[a-z_.]+)["'`]/g)].map((m) => m[1] as string);
}

function tsFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? tsFiles(p) : p.endsWith(".ts") ? [p] : [];
  });
}

describe("telemetry names", () => {
  const names = known();

  it("every span and attribute otel-emit sends is in otel.rs", () => {
    const used = [...Object.values(SPAN), ...Object.values(ATTR)];
    expect(used.filter((n) => !names.has(n))).toEqual([]);
    console.log(`checked ${used.length} otel-emit names against ${names.size} in otel.rs`);
  });

  it("every rubato.* literal in the skill scripts and plan-check is in otel.rs", () => {
    const files = [...tsFiles(join(root, "skills/rubato/scripts")), ...tsFiles(join(root, "actions/plan-check/src"))];
    const unknown: string[] = [];
    let count = 0;
    for (const f of files) {
      for (const n of namesIn(readFileSync(f, "utf8"))) {
        count++;
        if (!names.has(n)) {
          unknown.push(`${f.slice(root.length + 1)}: ${n}`);
        }
      }
    }
    expect(unknown).toEqual([]);
    expect(count).toBeGreaterThan(0);
    console.log(`checked ${count} rubato.* literals in ${files.length} files`);
  });

  it("fails on purpose for a name otel.rs does not have", () => {
    const fixture = readFileSync(join(import.meta.dirname, "fixtures/otel-bad-name.ts"), "utf8");
    expect(namesIn(fixture).filter((n) => !names.has(n))).toEqual(["rubato.skill.bogus"]);
  });
});
