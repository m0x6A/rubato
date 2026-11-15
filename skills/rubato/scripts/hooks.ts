#!/usr/bin/env -S node --experimental-strip-types
/**
 * Render `hooks.json` for one companion (ADR 0013, 0063). The companion does the same in Rust
 * at launch; this script is for a developer opting a manual session in. The token is never on a
 * command line: every hook runs `curl -H @<header file>`, and the companion writes that file
 * (mode 0600) in its state dir as `hook-auth.header`; `rubato-companion status` prints both.
 *
 *   hooks.ts --port <n> --header-file <state dir>/hook-auth.header [--out <file>]
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

/** Single-quote for `sh -c`, as the companion's `render_hooks` does. */
function shQuote(s: string): string {
  return `'${s.replaceAll("'", "'\\''")}'`;
}

export function renderHooks(template: string, port: number, headerFile: string): string {
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`port must be 1..65535, got ${port}`);
  }
  if (!headerFile) {
    throw new Error("--header-file is required: the companion's hook-auth.header (rubato-companion status prints it)");
  }
  // Shell-quoted, then escaped for its place inside a JSON string.
  const inJson = JSON.stringify(shQuote(headerFile)).slice(1, -1);
  const out = template.replaceAll("{port}", String(port)).replaceAll("{header_file}", inJson);
  JSON.parse(out);
  return out;
}

function opt(argv: string[], name: string): string | undefined {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
}

export function main(argv: string[], err: (s: string) => void = (s) => console.error(s)): number {
  if (argv.includes("--token")) {
    err("--token is no longer accepted: a token on a command line is readable by every local user (ADR 0063). Pass --header-file <state dir>/hook-auth.header; rubato-companion status prints its path.");
    return 2;
  }
  const port = Number(opt(argv, "--port"));
  const headerFile = opt(argv, "--header-file") ?? "";
  try {
    const rendered = renderHooks(readFileSync(templatePath(), "utf8"), port, headerFile);
    const out = opt(argv, "--out");
    if (out) {
      writeFileSync(out, rendered);
      console.log(`wrote ${out}`);
    } else {
      console.log(rendered);
    }
    return 0;
  } catch (e) {
    err(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/^.*\//, ""))) {
  process.exitCode = main(process.argv.slice(2));
}
