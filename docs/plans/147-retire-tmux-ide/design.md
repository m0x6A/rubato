# Design: retire tmux-ide (#147)

Plan: [../147.md](../147.md) · ADR: [0050](../../decisions/0050-retire-tmux-ide.md), proposed,
supersedes [0003](../../decisions/0003-claude-code-tmux-tmux-ide.md).

## 1. What tmux-ide does for us today

Every call the companion makes, from `crates/rubato-companion/src/kernel/tmux_ide.rs`:

| Call | Where | What we get | Replacement |
| --- | --- | --- | --- |
| `--version` | `launch/launcher.rs:89`, `stale/heartbeat.rs:27` | the pin (2.9.0) and `rubato.tmux_ide.version` | `tmux -V` against `companion.tmux_min_version` (3.7); `rubato.tmux.version` (§3) |
| `worktree create <b> --from <base> --no-session` | `launcher.rs:162` | a worktree at `<repo>-worktrees/<b>` | `gitx::worktree_add` (§2) |
| `adopt <session>` | `launcher.rs:187` | tmux-ide's status bar; failure ignored | nothing |
| `team --json` | `sessions/tasks.rs:146`, every `team_poll_interval_secs` | agent state for panes tmux-ide's own hooks stamped (`since`); screen-matched states we already render *unknown* (`sessions/discovery.rs:113`) | nothing; hooks are the only state source (§4) |
| `events --follow --json` | `sessions/tasks.rs:21` | a wake-up for reconcile, only for adopted sessions; exits at once on a fresh machine | nothing; the timer and the launch arm's wake-up (`launch/arm.rs:88`) stay |

ADR 0003 also names `send`, `wait`, MCP and the SDK. None of them is called anywhere
(the only `"send"` in `crates/` is the websocket sink, `server_link/client.rs:164`). Session creation is already
`tmux new-session -d` (`kernel/tmux.rs:120`), pane discovery is `tmux list-panes` plus `ps` for
wrapped agents (`sessions/tasks.rs:109`), and a launched session's events all come from our hooks
(ADR 0013).

## 2. Worktree without tmux-ide

The launcher already looks for an existing worktree on the branch (`launcher.rs:155`). When there
is none, `gitx::worktree_add(repo, branch, base_ref, timeout)` picks one of three commands, all at
`default_worktree_path(repo, branch)` so existing worktrees and the path in QUICKSTART stay put:

1. a local branch `<b>` exists: `git worktree add <path> <b>`;
2. else `origin/<b>` exists (pushed earlier from another machine): `git worktree add --track -b <b> <path> origin/<b>`;
3. else: `git worktree add -b <b> <path> <base_ref>`.

Each runs under `companion.command_timeout()` like every other git call (plan 40). A `worktree
add` killed at the deadline can leave a registration without a directory or a directory without
a registration. So `worktree_add` first runs `git worktree prune` (drops registrations whose
directory is gone), and if the path still exists without being a listed worktree it fails with
the path in the error. It never deletes a directory: the developer may have work in it. Which of these
tmux-ide did for cases 1 and 2 is not documented; A5 assumes the order above and
`worktree.feature` gains one scenario per case.

`default_worktree_path` moves from `kernel/tmux_ide.rs` to `kernel/gitx.rs` unchanged, with its
test.

## 3. Dependency check and version

The launch refuses with `missing_dependency` when `tmux -V` fails or reports a version below
`companion.tmux_min_version` (default `3.7`, the floor README already states); a hung `tmux -V` is
`Failed`, not refused, as for tmux-ide today (plan 40 A2). `Tmux::version()` parses `tmux 3.8`,
`tmux 3.7a` and `tmux next-3.9`. The heartbeat carries it (Q2), and the heartbeat span gets
`rubato.tmux.version`.

`companion.tmux_ide_command` and `companion.tmux_ide_min_version` go from `CompanionConfig`.
`local.yml` files that still set them keep parsing, since the config ignores unknown keys
(`kernel/config.rs:65`: `#[serde(default)]`, no `deny_unknown_fields`); the companion logs one `info!` naming the key so the developer can
delete it.

## 4. Agent state after tmux-ide

The registry's sources become `Hook > Launch > Scraped`. `Scraped` stays: it is what a pane
running Claude Code gets before any hook speaks (`discovery.rs:36`), and the name keeps every
stored snapshot readable, although nothing is scraped. `apply_fleet`, `Fleet` and `TeamPoll` go.

Who loses what:

- **A session Rubato launched**: nothing. Its hooks were already the fast, trusted path.
- **A session the developer started themselves with tmux-ide's hooks installed**: it showed
  *working*/*blocked* from tmux-ide; it now shows *unknown* until the developer opts it in with
  `hooks.ts --out .claude/settings.local.json` (SKILL.md "Don't"). Q3 asks whether that is enough.
- **A pane that closes**: seen at the next reconcile (30 s by default) rather than possibly at
  once through `events --follow`. `SessionEnd` from our hooks still ends a launched session at
  once.

The reconcile cycle gets its own span, `rubato.reconcile`, with `rubato.companion_id`, the pane
count and an error status when `tmux list-panes` fails. It replaces `rubato.team_poll`, which
`e2e/run.ts:344` expects today; the e2e list changes with it.

## 5. The wire (companion-server)

| Field | Today | After | Why |
| --- | --- | --- | --- |
| `StateSource::tmux_ide` | sent for panes with tmux-ide hook authority | still accepted, never sent; doc comment says so | `session_snapshots.sessions` is stored JSON (`migrations/0001_init.sql:71`); removing the value would make those rows fail to parse |
| `Heartbeat.tmux_ide_version` | set from `tmux-ide --version` | still accepted, never set | an older companion may still send it |
| `Heartbeat.tmux_version` | — | new, optional | the pin is now on tmux (Q2) |
| `Limits.team_poll_interval_secs` | interval of the `team --json` poll | same name, now the reconcile interval; doc comment changes | renaming it is a breaking wire change for no gain |
| `RefusalReason::MissingDependency` doc | "tmux-ide or claude missing…" | "tmux or claude missing…" | — |

The schema snapshots under `schema/protocol/` change in descriptions and gain `tmux_version`;
`web/src/protocol.ts` is regenerated. Nothing is removed from the wire in this change (Q2 b is the
alternative).

## 6. The map

Eight merged plans (4, 7, 20, 26, 27, 40, 41, v0) name `companion-tmux-ide`, and the plan check
runs every plan in `docs/plans/` against today's map (`.github/workflows/ci.yml:52`). Deleting the
node and the seam would fail CI on history. Q1 asks how to handle it; the recommendation keeps both
ids, marked retired, and stops drawing them as live.

## 7. Tests and CI

- `discovery.feature`: the `team --json` scenarios go (real tool parses; recorded fleet with
  `tmux_ide` source; events unavailable). The pane-discovered-untrusted scenario stays and becomes
  the suite's statement of the rule: state from hooks only.
- `launch-timeout.feature`: the hanging stand-in wraps tmux and hangs on `new-session` instead of
  wrapping tmux-ide on `worktree create`; the error text becomes `tmux new-session`. A second
  scenario hangs `git worktree add` through a `git` wrapper first on `PATH` (the same technique the
  tmux-ide wrapper uses today); this is a hang injected around the real git, not a mock of it.
- `worktree.feature`: three scenarios for §2, plus "a tmux below the pinned version is refused".
- `pairing.feature`: the heartbeat names the tmux version.
- `launch/arm.rs` unit tests: the slow stand-in becomes a slow `tmux` (`-V` sleeps 1 s), so the
  "registry lock not held" test still proves what it proves.
- `.github/workflows/ci.yml`: both `npm install -g tmux-ide` lines and `tmux-ide --version` go.
- Web tests that use `tmux-ide` only as a node id or as sample refusal text keep working while the
  node stays in the map (Q1 a); sample texts are changed to `tmux 3.7 is needed` anyway.

## 8. MCP, kept in mind

tmux-ide's MCP was the one capability we expected to want later: an agent asking which agent works
on what. After this change the place for it is `rubato-server`, which already holds the derived
state (sessions per issue, progress, plans, findings) in the overview. An MCP server there would be
a new consumer on the server side and a new seam to claude-code, and needs its own auth
(user-to-server tokens, ADR 0030). It is not on the companion: ADR 0004 forbids anything reaching a
developer's machine inbound, and the companion only sees its own user's sessions. Nothing in this
change makes that harder; it is a follow-up issue, not a task here.

## 9. PRs

0. `issue/147-retire-tmux-ide`: this note, the plan, ADR 0050 (proposed). Design review.
1. Protocol: `tmux_version`, doc comments, otel names (`rubato.tmux.version`, `rubato.reconcile`;
   `rubato.tmux_ide.version` and `rubato.team_poll` kept as deprecated constants until PR 2 lands),
   schema snapshots, `protocol.ts`.
2. Companion: §2–§4, features, unit tests, CI, README, QUICKSTART, AGENTS.md, `.claude/profiles/system-base.mmd`.
3. Map: per Q1.
