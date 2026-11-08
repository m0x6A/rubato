#!/usr/bin/env -S node --experimental-strip-types
/**
 * Render `hooks.json` for one companion (ADR 0013). The companion does the same in Rust at
 * launch; this script is for a developer opting a manual session in.
 *
 *   hooks.ts --port <n> --token <t> [--out <file>]
 */
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { skillRoot } from "./lib/vendor.ts";

export const HOOK_EVENTS = [
  "SessionStart",
  "UserPromptSubmit",
  "PreToolUse",
  "PostToolUse",
  "PostToolUseFailure",
  "Notification",
  "Stop",
  "StopFailure",
  "SessionEnd",
] as const;

export function templatePath(): string {
  return resolve(skillRoot, "hooks.json");
}

export function renderHooks(template: string, port: number, token: string): string {
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`port must be 1..65535, got ${port}`);
  }
  if (!/^[A-Za-z0-9_-]{16,}$/.test(token)) {
    throw new Error("token must be at least 16 URL-safe characters");
  }
  const out = template.replaceAll("{port}", String(port)).replaceAll("{token}", token);
  JSON.parse(out);
  return out;
}

function opt(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
}

function main(argv: string[]): number {
  const port = Number(opt(argv, "--port"));
  const token = opt(argv, "--token") ?? "";
  try {
    const rendered = renderHooks(readFileSync(templatePath(), "utf8"), port, token);
    const out = opt(argv, "--out");
    if (out) {
      writeFileSync(out, rendered);
      console.log(`wrote ${out}`);
    } else {
      console.log(rendered);
    }
    return 0;
  } catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = main(process.argv.slice(2));
}
