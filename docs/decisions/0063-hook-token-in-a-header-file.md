# 0063. Pass the hook token to curl in a 0600 header file, and read only capped transcripts under the Claude projects dir

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/167#issuecomment-6892512493 (self); drafted by agent (m0x6A/rubato#167)
- **Seam:** companion-claude-code
- **Issue:** m0x6A/rubato#167

## Context

ADR 0013 puts the per-start token in the hook command itself
(`curl -H "Authorization: Bearer <token>" ...`). Every local user can read a process's argv on
macOS, and on Linux without `hidepid`, so any local user can take the token from `ps` while a hook
runs and post forged hooks. A forged hook can name any `transcript_path`; the companion reads it
whole with no size or location check, so a fake transcript trips the spend cap (and the session
is killed) and `/dev/zero` grows the read until the companion runs out of memory. Plan:
`docs/plans/167.md`, design `docs/plans/167-hook-token-transcript/design.md`.

## Decision

The hook command no longer carries the token. Each launch writes `hook-auth.header` next to its
`hooks.json`, mode 0600, holding the one line `Authorization: Bearer <token>`, and the command is
`curl -sS -m 2 -H @'<header file>' -H 'Content-Type: application/json' --data-binary @- http://127.0.0.1:<port>/hook`.
The template in `skills/rubato/hooks.json` has `{port}` and `{header_file}` placeholders (no
`{token}`); the rendered path is shell-quoted. The companion also writes `hook-auth.header` in its
state dir at start for `render-hooks` and `hooks.ts --header-file`, which opt a manual session in
without the token on any command line. The receiver compares the token in constant time.

The companion reads a hook's `transcript_path` for spend only when it resolves (symlinks
followed) to a regular file under `$CLAUDE_CONFIG_DIR/projects`, or `~/.claude/projects` when
that is unset, and only up to 256 MiB; otherwise it logs `rubato.transcript.refused` with the
reason and counts `rubato.events.dropped` with `rubato.reason = transcript`, and keeps the
session's last spend.

Everything else in ADR 0013 stands: loopback HTTP, a per-start token, `async` hooks, the
template injected with `--settings`.

## Alternatives considered

- **curl `--config <file>`**: also keeps argv clean, but the config file then holds the URL and
  every flag, so the template no longer shows the command; `-H @file` changes one argument.
- **A Unix socket in a 0700 directory**: filesystem permissions instead of a token, but it
  replaces the transport ADR 0013 chose and needs `curl --unix-socket` and a socket path per
  start; more change than the leak needs.
- **Token in an environment variable**: `/proc/<pid>/environ` is owner-only on Linux, but
  `ps eww` shows the environment on macOS.
- **Accept any transcript path, only cap the size**: still lets a forged or mistaken path make
  the companion read an arbitrary file of the user's.

## Consequences

Easier: the token is never on a command line, in the companion's process list or in a hooks
file; a forged hook can no longer point the spend reader at an arbitrary file. Harder: curl must
support `-H @file` (7.55.0, 2017); a manual session needs the state dir's header file path
instead of the token; a transcript kept outside the Claude projects dir (a non-default layout,
a test stand-in) gets no spend. Watch: `rubato.events.dropped` by `rubato.reason` (`auth`,
`transcript`) on the Events dropped panel, and `SessionWorkingNoHooks` if the new command stops
delivering. This is the `companion-claude-code` O-row of plan 167.

## Supersedes

0013 (the hook command and its `{token}` placeholder only)
