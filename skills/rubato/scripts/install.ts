#!/usr/bin/env -S node --experimental-strip-types
/**
 * Install the skill at user level (`~/.claude/skills/rubato`) with the plan-check sources and
 * schemas vendored, then `npm install --omit=dev` there. Run from a rubato checkout.
 *
 *   install.ts [--to <dir>] [--no-npm]
 */
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { skillRoot } from "./lib/vendor.ts";

export function install(target: string, runNpm: boolean): string {
  const repoRoot = resolve(skillRoot, "..", "..");
  rmSync(target, { recursive: true, force: true });
  mkdirSync(target, { recursive: true });
  for (const f of ["SKILL.md", "hooks.json", "prompt.md", "decision-template.md", "review-template.md", "review-checklist.md", "local.example.yml", "scripts"]) {
    cpSync(join(skillRoot, f), join(target, f), { recursive: true });
  }
  cpSync(join(repoRoot, "actions", "plan-check", "src"), join(target, "vendor", "plan-check"), { recursive: true });
  mkdirSync(join(target, "schema"), { recursive: true });
  for (const f of ["system-map.schema.json", "plan.schema.json"]) {
    cpSync(join(repoRoot, "schema", f), join(target, "schema", f));
  }
  const pkg = JSON.parse(readFileSync(join(skillRoot, "package.json"), "utf8")) as { dependencies: Record<string, string> };
  writeFileSync(join(target, "package.json"), JSON.stringify({ name: "rubato-skill", private: true, type: "module", dependencies: pkg.dependencies }, null, 2) + "\n");
  if (runNpm) {
    execFileSync("npm", ["install", "--omit=dev", "--no-audit", "--no-fund"], { cwd: target, stdio: "inherit" });
  }
  return target;
}

function main(argv: string[]): number {
  const i = argv.indexOf("--to");
  const target = resolve(i >= 0 ? (argv[i + 1] ?? "") : join(homedir(), ".claude", "skills", "rubato"));
  try {
    install(target, !argv.includes("--no-npm"));
    console.log(`installed rubato skill to ${target}`);
    return 0;
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = main(process.argv.slice(2));
}
