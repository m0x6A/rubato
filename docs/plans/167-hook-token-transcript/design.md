# Design: keep the hook token out of argv; read only real transcripts, capped (#167)

Plan: [../167.md](../167.md) · ADR 0050 (*proposed*, supersedes part of 0013).

## The threat

Another local OS user on a developer machine (a shared WSL host, a CI runner, a second account
on a Mac). Not the developer, not the server, not a remote attacker: the receiver listens on
127.0.0.1 only. Whether such machines exist on the team is the open question in the issue; this
change is cheap enough to make without knowing.

What that user can do today, traced at e36d1f3:

1. Read the token from `ps` while any hook runs: `skills/rubato/hooks.json:4` puts it in the
   curl argv; `hooks/template.rs:9-15` fills it in.
2. Post forged hooks: `hooks/server.rs:80-90` checks only that token, with `!=`.
3. Point an existing session at any file: `hooks/apply.rs:20-22` takes `transcript_path` as is.
4. Make the companion read it whole: `spend/arm.rs:24-27` → `spend/sum.rs` `read_to_string`.
   A fake transcript with huge usage passes the cap and `spend/arm.rs:31-52` kills the real tmux
   session; `/dev/zero` grows the read until the companion dies.

## The fix, part by part

### D1. The token travels in a 0600 header file (seam companion-claude-code, Q1)

`curl -H @<file>` reads header lines from a file (curl 7.55.0+). The launcher writes
`<state>/launches/<id>/hook-auth.header` with `write_private` (0600), one line
`Authorization: Bearer <token>`, next to `hooks.json`. The template has `{header_file}` in place
of `{token}`; `render_hooks(template, port, header_file)` substitutes a shell-quoted path
(`'…'`, with `'` written `'\''`), JSON-escaped, and parses the result as JSON as before. No
rendered command contains the token, so the hooks file no longer holds a secret (it stays 0600:
it names the port and the header path, and `write_private` costs nothing).

What tmux-ide and Claude Code need keeps working: Claude Code runs the command with `sh -c`
and the body on stdin, unchanged; tmux-ide's own user-level hooks merge under `--settings` as
ADR 0013 verified; nothing in tmux-ide reads our hooks.

Manual opt-in (`hooks.ts`, `rubato-companion render-hooks`): the companion writes
`<state>/hook-auth.header` next to `hook-endpoint.json` when it starts (`HookEndpoint::save`).
`hooks.ts` takes `--port <p> --header-file <path>` and refuses `--token` with a message naming
the header file, so no token reaches a command line there either. `render-hooks` uses the state
dir's file. `rubato-companion status` prints the header file's path.

The token stays per start. A session launched before a restart posts to a dead port either way
(ADR 0013's lost-events gap, covered by `SessionWorkingNoHooks`).

### D2. Constant-time comparison

`hooks/server.rs` compares the presented token with the expected one by folding the XOR of
every byte, after a length check (the length is public: 32 characters). A dozen lines, no crate
(a crate would make the track full on its own and buys nothing here).

### D3. Transcripts: where, what kind, how big (seam companion-claude-code, Q2)

New `spend/transcript.rs`, used by `sum_file`:

- **Root.** `$CLAUDE_CONFIG_DIR/projects` if set, else `$HOME/.claude/projects`, read from the
  companion's environment, canonicalized. Claude Code writes transcripts there; the hook fixtures
  (`features/fixtures/hooks/*.json`) show the shape. If the root does not exist, every path is
  refused.
- **Location.** The hook's path is canonicalized (symlinks resolved, `..` removed) and must
  start with the root. A symlink from inside the root to outside is refused, because the
  resolved path is what is checked.
- **Kind.** The file is opened and its metadata read from the open handle: it must be a regular
  file. `/dev/zero`, a FIFO and a directory are refused before any byte is read.
- **Size.** At most 256 MiB by metadata, and the read is `take(cap + 1)` so a file that grows
  between the check and the read is still bounded.

A refusal is one `warn` line `rubato.transcript.refused` with `rubato.reason`
(`outside_root`, `not_a_file`, `too_large`, `unreadable`) and the path, and one
`rubato.events.dropped` with `rubato.reason = transcript`. The session keeps its last spend; no
cap decision is made from a refused file.

Left as it is: the whole file is read on every hook (as today). Reading incrementally from the
last offset would make spend O(new lines) per hook; it is a separate change (the dedupe by
message id in `sum_text` would need its state kept per session). Named in the PR as a follow-up.

Also left: `hooks/apply.rs` still records the hook's `transcript_path` on the entry. With the
token out of argv only Claude Code can post hooks, and the read is checked where it happens,
which also covers a path recorded before this change.

### D4. The e2e stand-in

`e2e/claude-standin.ts` writes its transcript under `$HOME/.claude-standin/transcripts`, which D3
refuses. It moves to `$CLAUDE_CONFIG_DIR/projects/<slug>/` or `$HOME/.claude/projects/<slug>/`,
the layout Claude Code uses, so the e2e run's "spend from the transcript" check keeps passing. It
already runs every hook command with `sh -c`, so it exercises the header file end to end.

## What we watch

- `rubato.events.dropped{rubato.reason="auth"}`: forged or broken posts (unchanged reason).
- `rubato.events.dropped{rubato.reason="transcript"}` and the `rubato.transcript.refused` line:
  a transcript refused. A non-default Claude layout shows here, not as silent zero spend.
- `SessionWorkingNoHooks`: if `-H @file` fails on some machine (old curl, unreadable file), hooks
  stop and this alert fires as it does today for a dead port.
