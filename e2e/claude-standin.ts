#!/usr/bin/env -S node --experimental-strip-types
/**
 * A stand-in for Claude Code in the end-to-end run. It does what a launched session does on
 * the outside: runs the hooks from the `--settings` file exactly as Claude Code would (the
 * command, the stdin JSON), writes the plan at the path named in the prompt, commits and
 * pushes it, writes a transcript with usage, then stays in the foreground.
 *
 *   claude --session-id <uuid> --settings <hooks.json> --permission-mode <m> "<prompt>"
 */
import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const argv = process.argv.slice(2);
const opt = (name: string) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
};
const sessionId = opt("--session-id") ?? "no-session";
const settingsPath = opt("--settings");
const prompt = argv.filter((a, i) => !a.startsWith("--") && argv[i - 1] !== "--session-id" && argv[i - 1] !== "--settings" && argv[i - 1] !== "--permission-mode").join(" ");
const cwd = process.cwd();
const log = (line: string) => appendFileSync(resolve(cwd, ".claude-standin.log"), `${line}\n`);

log(`ARGS: ${argv.join(" ")}`);
for (const [k, v] of Object.entries(process.env)) {
  if (/^(OTEL_|CLAUDE_|RUBATO_)/.test(k)) {
    log(`${k}=${v}`);
  }
}

const transcriptDir = resolve(process.env["HOME"] ?? "/tmp", ".claude-standin", "transcripts");
mkdirSync(transcriptDir, { recursive: true });
const transcriptPath = resolve(transcriptDir, `${sessionId}.jsonl`);
writeFileSync(transcriptPath, "");

interface HookEntry { hooks: { type: string; command: string }[] }
const settings = settingsPath ? (JSON.parse(readFileSync(settingsPath, "utf8")) as { hooks: Record<string, HookEntry[]> }) : { hooks: {} };

/** Run every hook command for an event with the payload on stdin, as Claude Code does. */
function fire(event: string, extra: Record<string, unknown> = {}) {
  const payload = JSON.stringify({ session_id: sessionId, transcript_path: transcriptPath, cwd, permission_mode: opt("--permission-mode") ?? "default", hook_event_name: event, ...extra });
  for (const entry of settings.hooks[event] ?? []) {
    for (const h of entry.hooks) {
      if (h.type !== "command") {
        continue;
      }
      const r = spawnSync("sh", ["-c", h.command], { input: payload, encoding: "utf8" });
      log(`HOOK ${event} exit=${r.status}`);
    }
  }
}

function usage(input: number, output: number) {
  appendFileSync(transcriptPath, JSON.stringify({ type: "assistant", requestId: `r${Date.now()}`, message: { id: `msg_${Date.now()}`, model: "claude-standin-5", usage: { input_tokens: input, output_tokens: output, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 } } }) + "\n");
}

// A real session thinks between tool calls; pause so the progress model can be observed moving.
const pause = Number(process.env["RUBATO_STANDIN_DELAY_MS"] ?? "1500");
// Blocks without using a processor: three sessions spinning at once starved the server and the
// companion on a small CI runner. The process still stays in the pane's foreground.
const sleep = (ms: number) => void Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

fire("SessionStart", { source: "startup" });
fire("UserPromptSubmit", { prompt });
usage(1200, 300);

// The plan: template chosen by issue, path from the prompt ("write the plan at <path>,").
const issue = /([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+#\d+)/.exec(prompt)?.[1] ?? "";
if (!issue) {
  log(`ERROR: no owner/name#number in prompt: ${prompt}`);
  process.exit(2);
}
const planPath = /plan at (\S+?),/.exec(prompt)?.[1] ?? "docs/plans/unknown.md";
const templates = process.env["RUBATO_STANDIN_PLANS"] ?? "";
const templateFile = resolve(templates, `${issue.replace("/", "-").replace("#", "-")}.md`);
sleep(pause);
fire("PreToolUse", { tool_name: "Write", tool_input: { file_path: planPath } });
mkdirSync(dirname(resolve(cwd, planPath)), { recursive: true });
writeFileSync(resolve(cwd, planPath), readFileSync(templateFile, "utf8"));
fire("PostToolUse", { tool_name: "Write", tool_input: { file_path: planPath }, tool_response: { ok: true } });
usage(800, 1500);

sleep(pause);
fire("PreToolUse", { tool_name: "Bash", tool_input: { command: "git add && git commit && git push" } });
const git = (...a: string[]) => execFileSync("git", ["-c", "user.name=standin", "-c", "user.email=standin@acme.test", ...a], { cwd, encoding: "utf8" });
git("add", planPath);
git("commit", "-q", "-m", `Add plan for ${issue}`);
git("push", "-q", "-u", "origin", "HEAD");
fire("PostToolUse", { tool_name: "Bash", tool_input: { command: "git push" }, tool_response: { stdout: "pushed" } });
usage(400, 200);

sleep(pause);
fire("Stop", { stop_hook_active: false, last_assistant_message: `Plan written to ${planPath} and pushed.` });
log("DONE");
// Stay alive like an idle session until the companion or the run ends.
setInterval(() => {}, 60_000);
