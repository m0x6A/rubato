# Design: retire tmux-ide (#147)

Plan: [../147.md](../147.md) · ADR: [0070](../../decisions/0070-retire-tmux-ide.md), proposed,
supersedes 0003 and 0013 in part.

**Refreshed on 2026-11-15** against main e4c58d8. The first version was approved as plan 62db41a
on 2026-11-14, with its ADR numbered 0050. Since then, main gained:
- #122's headless `claude -p` turns (ADR 0062), which already make worktrees with git;
- session_input's tmux paste (ADR 0051);
- #192's `--` before tmux-ide arguments;
- #194 (TLS off loopback) and #172 (spend off the dispatch loop, `Limits` over `watch`);
- #197's agent MCP in the companion (ADR 0071), whose follow-ups #220–#230 land after this.

§1–§9 say where each of these changes the design.

## 1. What tmux-ide does for us today

Every call the companion makes, from `crates/rubato-companion/src/kernel/tmux_ide.rs`. An
interactive companion makes all five; a headless one makes none (ADR 0062).

| Call | Where | What we get | Replacement |
| --- | --- | --- | --- |
| `--version` | `launch/launcher.rs:97`, `stale/heartbeat.rs:27` | the pin (2.9.0) and `Heartbeat.tmux_ide_version` | `tmux -V` against `companion.tmux_min_version` (3.7) and `Heartbeat.tmux_version` (§3) |
| `worktree create --from <base> --no-session -- <b>` | `launcher.rs:185` | a worktree at `<repo>-worktrees/<b>` | `gitx::worktree_add`, already used by headless mode at `launcher.rs:173`, extended (§2) |
| `adopt <session>` | `launcher.rs:253` | tmux-ide's status bar; a failure is ignored | nothing |
| `team --json` | `sessions/tasks.rs:67`, every `team_poll_interval_secs` | agent state for panes tmux-ide's own hooks stamped (`since`); screen-matched states already render *unknown* (`sessions/discovery.rs:122`) | nothing: hooks are the only state source (§4) |
| `events --follow --json` | `sessions/tasks.rs:21` | a wake-up for reconcile, only for adopted sessions; it exits at once on a fresh machine | nothing: the timer and the launch arm's wake-up (`launch/arm.rs:138`) stay |

ADR 0003 also names `send`, `wait`, MCP and the SDK:
- `send` was replaced by a tmux paste (ADR 0051, `kernel/tmux.rs`, the `SessionInput` port). It
  does not go through tmux-ide.
- `wait` and the SDK are called nowhere.
- MCP is now ADR 0071's, served by the companion itself (§8).

The rest is already done without tmux-ide:
- session creation is `tmux new-session -d` (`kernel/tmux.rs:169`);
- pane discovery is `tmux list-panes` plus `ps` for wrapped agents (`sessions/tasks.rs:111`);
- a launched session's events all come from our hooks (ADR 0013), and a headless turn's from its
  own process (ADR 0062).

**The issue's question 2, anything we'd want later.**
- `send` is already replaced (ADR 0051), and #91's waiting for an approval arrives as a typed
  prompt (ADR 0055).
- #134's orchestration across repositories is a non-goal in AGENTS.md; its agent-only clients
  are ADR 0062's headless mode, which needs no tmux-ide.
- tmux-ide's MCP is replaced by ADR 0071's.

So keeping tmux-ide saves no later build.

**The issue's question 4, "stitch, don't build".** It still holds. We stitch tmux, git and Claude
Code's hooks, three tools every machine already has, and drop a wrapper around them. The code
the companion gains is the three `git worktree add` forms and a version parse.

## 2. Worktree without tmux-ide

The launcher already looks for an existing worktree on the branch (`launcher.rs:168`). When there
is none:
- a **headless** companion calls `gitx::worktree_add` (`kernel/gitx.rs:103`) today, with two
  cases: the local branch, else a new one from the base ref;
- an **interactive** companion calls tmux-ide.

After this change, both call the same `gitx::worktree_add(repo, path, branch, base_ref,
timeout)`, at `default_worktree_path(repo, branch)`, so existing worktrees and the path in
QUICKSTART stay put. It picks one of three commands:

1. a local branch `<b>` exists: `git worktree add -- <path> <b>`;
2. else `refs/remotes/origin/<b>` exists (pushed earlier from another machine; the launch has just
   run `git fetch`): `git worktree add --track -b <b> -- <path> origin/<b>`;
3. else: `git worktree add --no-track -b <b> -- <path> <base_ref>`.

**Upstream.** Without `--no-track`, case 3 sets the new branch's upstream to the base ref's
remote branch, for example `origin/main`, which is what headless mode does today. A bare
`git push` from the agent's worktree is then refused under `push.default=simple`. With it, the
branch has no upstream until the agent's first `git push -u origin <branch>`. Case 2 tracks
`origin/<b>`, which is the branch itself.

**Which git.** A new `companion.git_command` (default `git`) is threaded through `gitx`, as
`tmux_command` is through `Tmux`. Tests swap it for a wrapper around the real git, the way they
swap `tmux_ide_command` today. A wrapper first on `PATH` would also catch the test harness's own
git calls.

**Command safety.** The branch and the base ref come from the server. #192 (issue #155) put a `--`
before tmux-ide's positional arguments for that reason. Here:
- `-b` takes the branch as its value;
- `--` ends the options before `<path>` and the commit-ish;
- `check_branch` and `check_ref` (`launch/script.rs`) run first, as today, so a branch is always
  `issue/<n>-<slug>` for the command's issue.

**Deadline.** Each git call runs under `companion.command_timeout()`, like every other launch call
(plan 40).

**Leftovers.** A `worktree add` killed at the deadline can leave a registration without a
directory, or a directory without a registration. Each is handled on its own:

- **A registration without a directory.** `git worktree list --porcelain` still lists it, with
  its branch, marked `prunable`. So the launcher's lookup at `launcher.rs:168` would take it and
  start tmux in a directory that does not exist. Before the lookup, a `prunable` entry for the
  launch's own branch is removed with `git worktree remove -- <path>`, with a `warn!` naming the
  path. `gitx::worktrees` reports `prunable` for that. Nothing repository-wide runs: a
  `git worktree prune` would also drop the developer's other worktrees, for example those on an
  unmounted disk.
- **A directory without a registration.** If the path exists without being a listed worktree,
  `worktree_add` fails with the path in the error. It never deletes a directory: the developer
  may have work in it.

Which of these tmux-ide did for cases 1 and 2 is not documented. A5 assumes the order above, and
`worktree.feature` gains one scenario per case.

**Code moves.** `default_worktree_path` moves from `kernel/tmux_ide.rs` to `kernel/gitx.rs`
unchanged, with its test. Headless mode gets case 2, `--no-track`, the leftover check and the
stale-registration removal for free.

## 3. Dependency check and version

An interactive launch refuses with `missing_dependency` when `tmux -V` fails or reports a version
below `companion.tmux_min_version` (default `3.7`, the floor README already states). A hung
`tmux -V` is `Failed`, not refused, as for tmux-ide today (plan 40 A2). A headless launch checks
no tmux (ADR 0062), as it checks no tmux-ide today (`launcher.rs:96`).

`Tmux::version()` parses `tmux 3.8`, `tmux 3.7a` and `tmux next-3.9`. `version_at_least` moves
to `kernel/tmux.rs` and learns to compare a letter suffix as patch level 0.

**The launcher's tmux calls** (`-V`, `has-session`, `new-session`) run under
`companion.command_timeout()`, as tmux-ide's calls did (`runtime.rs:179`, `with_timeout`).
Reconcile's `list-panes` keeps `proc::LOCAL_TIMEOUT`. So the launch-timeout scenario that hangs
`tmux new-session` fails at the same 5 s deadline the tmux-ide one uses today.

**The heartbeat** carries `tmux_version`, read once at start like `tmux_ide_version` today
(`stale/heartbeat.rs:27`). A headless companion leaves it `None`. The start log line names it:
`info!(rubato.tmux.version, "tmux found")`.

**Config.** `companion.tmux_ide_command` and `companion.tmux_ide_min_version` go from
`CompanionConfig`. `local.yml` files that still set them keep parsing, because the config ignores
unknown keys (`kernel/config.rs:65`: `#[serde(default)]`, no `deny_unknown_fields`). The companion
logs one `info!` per retired key, so the developer can delete it.

## 4. Agent state after tmux-ide

The registry's ranks become `Hook = Headless > Launch > Scraped` (`sessions/discovery.rs:11`);
`TmuxIde` leaves the ranking. `Scraped` stays:
- it is what a pane running Claude Code gets before any hook speaks;
- the name keeps every stored snapshot readable, although nothing is scraped.

`apply_fleet`, `Fleet`, `Agent` and `TeamPoll` go, and so do `watch_events` and
`kernel/tmux_ide.rs`.

**The protocol keeps the variant.** `StateSource::ALL` in the protocol keeps `TmuxIde`, because
`all_sources_cover_every_variant_once` (`rubato-protocol/src/session.rs:351`) holds it equal to
the schema. The server's progress test loops over `ALL`, so it keeps covering a source that
stored rows and older companions still send. Only the companion's `record_gauges`
(`sessions/tasks.rs`) and `counts_by_source` (`sessions/registry.rs`) skip it (A6).

**The `rubato.launch` line.** It is an `info!` line, not a span (`launch/arm.rs:115`). Its
`outcome` field prints `Discriminant(n)` today; it becomes `started`, `refused` or `failed`, the
same word `rubato.launches` counts under `rubato.outcome`.

Who loses what:

- **A session Rubato launched, interactive or headless**: nothing. Its hooks, or its turn
  process, were already the fast, trusted path. ADR 0071's MCP report is not state either way.
- **A session the developer started themselves, with tmux-ide's hooks installed**: it showed
  *working* or *blocked* from tmux-ide. It now shows *unknown* until the developer opts it in with
  `hooks.ts --out .claude/settings.local.json` (SKILL.md "Don't"). Q3 settled that this is enough
  for now.
- **A pane that closes**: seen at the next reconcile (30 s by default), rather than possibly at
  once through `events --follow`. `SessionEnd` from our hooks still ends a launched session at
  once.

**The reconcile span.** Each reconcile cycle gets its own span, `rubato.reconcile`, with
`rubato.companion_id`, the pane count, and an error status when `tmux list-panes` fails. It
replaces `rubato.team_poll`, which `e2e/run.ts:357` expects today; the e2e list changes with it.
The cycle body keeps #172's shape: limits over `watch`, snapshots compared as structs off the
lock.

## 5. The wire (companion-server)

| Field | Today | After | Why |
| --- | --- | --- | --- |
| `StateSource::tmux_ide` | sent for panes with tmux-ide hook authority | still accepted, never sent; a plain `//` comment says so, so its schema description does not change | `session_snapshots.sessions` is stored JSON; removing the value would make those rows fail to parse. The server's read path turns a row that does not parse into an empty list (`companion_link/store.rs:49`), so the guard test asserts the session comes back |
| `Heartbeat.tmux_ide_version` | set from `tmux-ide --version` | still accepted, never set | an older companion may still send it |
| `Heartbeat.tmux_version` | — | new, optional | the pin is now on tmux (Q2) |
| `Limits.team_poll_interval_secs` | interval of the `team --json` poll | same name, now the reconcile interval; the doc comment changes | renaming it is a breaking wire change for no gain |
| `RefusalReason::MissingDependency` doc | "tmux-ide or claude missing…" | "tmux or claude missing…" | — |

The schema snapshots under `schema/protocol/` change: `heartbeat` and `companion_envelope` gain
`tmux_version`, and the descriptions change in `limits`, `server_envelope` and `launch_result`.
`overview` and `issue_summary` stay as they are, since `StateSource`'s documentation does not
change. `web/src/protocol.ts` is regenerated. Nothing is removed from the wire in this change (Q2 b is the
alternative). No capability is needed: an older server ignores the new optional field.

## 6. The map

Nine merged plans name `companion-tmux-ide`: 4, 7, 20, 26, 27, 40, 41, 155 and v0. The plan check
runs every plan in `docs/plans/` against today's map (`.github/workflows/ci.yml`), so deleting the
node and the seam would fail CI on history. Q1 settled it:
- keep both ids, with descriptions saying retired by ADR 0070;
- remove the seam's flows;
- drop step 3 of the `start-a-session` scenario;
- leave the rest of the scenario playing in the web e2e.

## 7. Tests and CI

- **`discovery.feature`**: the three tmux-ide scenarios go (the real tool parses; a recorded fleet
  with `tmux_ide` source; events unavailable), and so does `features/fixtures/team.json`. The
  pane-discovered-untrusted scenario stays and becomes the suite's statement of the rule: state
  from hooks only. A new scenario posts a hook for that pane's cwd and sees the entry turn
  `working` from source `hook`.
- **`launch-timeout.feature`**:
  - The hanging stand-in wraps tmux and hangs on `new-session`, instead of wrapping tmux-ide on
    `worktree create`. The error text becomes `tmux new-session`.
  - A second scenario hangs `git worktree add`. `companion.git_command` points at a wrapper that
    hangs on `worktree add` and passes everything else to the real git, as the tmux-ide wrapper is
    injected through `tmux_ide_command` today (`steps/launch_timeout.rs:44`). It is a hang
    injected around the real git, not a mock of it.
- **`worktree.feature`**:
  - three scenarios for §2;
  - the leftover-directory scenario;
  - the stale-registration scenario, with a second stale registration for another branch that
    must survive;
  - "a tmux below the pinned version is refused".
- **`gitx` unit tests**, in a temporary repo with a bare origin, with `git_command` pointed at a
  recording wrapper:
  - the argv has `--` before the path;
  - case 3 has no upstream;
  - case 2 tracks `origin/<b>`.
- **`headless.feature`** and **`launch-branch.feature`**: wording only ("tmux unusable", "goes to
  git"), including the step text at `steps/headless.rs:28`. Headless runs gain case 2 of §2
  through the shared function, covered by the worktree scenarios and the `gitx` tests.
- **`pairing.feature`**: the heartbeat names the tmux version. A `stale::heartbeat` unit test sees
  `tmux_version: None` in headless mode.
- **`launch/arm.rs` unit tests**: the slow stand-in becomes a slow `tmux` (`-V` sleeps 1 s), so
  the "registry lock not held" test still proves what it proves.
- **`sessions/tasks.rs` unit tests**: the two `team --json` tests become the reconcile span test.
  Its `tmux` stand-in succeeds on `list-sessions` and exits 1 on `list-panes`: a failing
  `list-sessions` reads as no server and an empty pane list (`kernel/tmux.rs:141`), not an error.
- **`e2e/run.ts`**: the expected spans change (`rubato.reconcile` for `rubato.team_poll`). The
  expected metrics gain `rubato.launches`, and the expected attribute keys gain `rubato.outcome`.
- **`.github/workflows/ci.yml`**: both `npm install -g tmux-ide` lines, `TMUX_IDE_VERSION` and
  `tmux-ide --version` go. This is a test workflow, not one that publishes.
- **Web tests** that use `tmux-ide` only as a node id keep working while the node stays in the map
  (Q1). Sample refusal texts change to `tmux 3.7 is needed` anyway.

## 8. MCP (ADR 0071)

The first version of this note put a later MCP server on `rubato-server`. ADR 0071 decided
otherwise:
- each companion serves MCP on its loopback receiver at `/mcp`, to the sessions it launched;
- it offers three tools, and the launch id is the identity;
- its alternatives name this note's §8 and say why.

So this change builds nothing for MCP, and takes nothing from it:
- tmux-ide's own MCP was never called;
- 0071's MCP does not read tmux-ide;
- manual sessions get no MCP, because a manual session has no launch id (0071 §7, the plan's Q3).

**Order against #220–#230.** #197's plan lands #147's companion PR before:
- #222, `/mcp` on the hook receiver;
- #223, `mcp.json` and `--mcp-config` in `launch.sh` and the turn script;
- #225, `AgentReport` and `PlanStatus` in `server_link`.

To keep the two from fighting, this change leaves alone:
- the hook receiver's routes (`hooks/`);
- the `claude` argument list in `launch/script.rs` (`launcher_script`) and in the headless turn
  script;
- `sessions/launches.rs`'s `LaunchRecord`;
- `server_link/`.

In `launch/launcher.rs` it changes only the dependency check and the worktree branch of `start`.
In `runtime.rs` it changes only the `TmuxIde` value, the events watcher and reconcile's arguments.
In `stale/heartbeat.rs` it changes only the version field.

## 9. PRs

1. **`issue/147-retire-tmux-ide-2`**: this note, the plan and ADR 0070 (proposed, accepted with
   the plan). After approval, on the same branch, the protocol change:
   - `tmux_version` and the doc comments;
   - the otel names `rubato.tmux.version`, `rubato.reconcile` and `rubato.launches` (by the
     existing `rubato.outcome` and `rubato.reason`), with `rubato.tmux_ide.version` and `rubato.team_poll` kept as
     deprecated constants until PR 2 lands;
   - the schema snapshots and `protocol.ts`.
2. **`issue/147-companion`**: §2–§4 and §7, plus:
   - the dashboard panel and its check;
   - README, QUICKSTART, AGENTS.md (scope, architecture, kernel block, "Decisions already made",
     "Don't") and `.claude/profiles/system-base.mmd`;
   - the crate description in `Cargo.toml`.
3. **`issue/147-map`**: the map per Q1; the server golden `overview.json`, the plan-check, skill
   and web tests that pin the real map re-recorded; the deprecated otel constants removed.
