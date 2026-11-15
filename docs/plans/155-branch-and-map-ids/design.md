# Design: issue branches the companion accepts, `--` to tmux-ide, and a character rule for map ids (#155, part 2)

Plan: [../155.md](../155.md). Folds in the two Companion bullets of #170 that close the routes
#155 describes: the branch passed to `tmux-ide worktree create` without `--`, and map node ids
with no character rule.

## 1. What is already on main (PR #156)

| Where | What |
| --- | --- |
| `crates/rubato-companion/src/launch/script.rs:13` | `sh_quote`: `'` → `'\''`, used for every value in `launch.sh` |
| `crates/rubato-companion/src/launch/script.rs:21` | `check_ref`: letters, digits and `._/-`, no `..`, no leading `-` or `/`, no `//`, no `.lock` |
| `crates/rubato-companion/src/launch/launcher.rs:149-150` | `check_ref` on branch and base ref before the worktree; a refusal is `LaunchResult::Failed` |
| `crates/rubato-server/src/launch/prompt.rs:32` | `is_slug`: letters and digits in words joined by single hyphens, at most 100 characters |
| `crates/rubato-server/src/launch/handler.rs:119` | 422 on `POST /api/launch` for a slug that is not `is_slug` |

So the shell route of #155 is closed. Three things remain:

1. The companion still accepts any branch `check_ref` passes, for example `main` or
   `issue/9-x` on a command for issue 7. ADR 0006 and `LaunchCommand.branch`'s own doc
   (`crates/rubato-protocol/src/launch.rs:20`) say it is `issue/<n>-<slug>`.
2. Positionals go to tmux-ide with no `--` (`crates/rubato-companion/src/kernel/tmux_ide.rs:160-170`,
   `:183`). `check_ref` refuses a leading `-` today, but the guard is in the caller, not at the
   call.
3. A map node or seam id can hold any character (`crates/rubato-protocol/src/map.rs:94-95,109-110`,
   `schema/system-map.schema.json:99`). Only the draft editor checks ids
   (`crates/rubato-server/src/map_draft/apply.rs:6`). A map merged by hand can carry a `'` into
   every launch pinned to that node (`OTEL_RESOURCE_ATTRIBUTES`, `rubato.node`). The quoting
   holds, but an id is also a URL segment, a CSS selector in the web and a Prometheus label
   value, so one rule at the source is cheaper than escaping in each consumer.

## 2. The change

### 2.1 One slug rule, in the protocol

`is_slug` moves from the server to `rubato_protocol::ids::is_branch_slug`, unchanged. Next to
it, `IssueRef::is_branch_for(&self, branch) -> bool` holds when the branch is
`issue/<self.number>-<slug>` and the slug passes `is_branch_slug`. The server's `is_slug`
becomes a re-export, so its 422 and its tests stay as they are.

The companion calls `is_branch_for` in place of `check_ref("branch", …)`
(`launcher.rs:149`, a one-line change). `check_ref` stays for the base ref. A branch that fails
is `LaunchResult::Failed` with "branch … is not issue/<n>-<slug> for <repo>#<n>", the same
outcome `check_ref` gives today, before any `git fetch`, worktree or tmux session.

The rule is stricter than `check_ref` in every way that matters: no `/` after the prefix, no `.`,
no `_`, so `..` and `.lock` cannot occur, and the worktree path built from the branch
(`tmux_ide.rs:203`) is always `<repo>-worktrees/issue/<n>-<slug>`.

Which letters a slug may hold is Q1. The code takes option (a), the rule on main.

### 2.2 `--` before positionals to tmux-ide

tmux-ide parses its arguments with Node's `util.parseArgs` (read in the published 2.9.24
package, `bin/cli.js`: `parseArgs4({ allowPositionals: true, strict: false, … })`), and
`parseArgs` treats everything after `--` as positional. So:

- `worktree create --from <base> --no-session -- <branch>`
- `adopt -- <session>`

`team --json`, `events --follow --json` and `--version` take no value from outside and are
unchanged. The calls in `kernel/gitx.rs` (`fetch`, `worktree list`) take no value from the
server, so they are out of scope.

### 2.3 Resource attribute keys

`OTEL_RESOURCE_ATTRIBUTES` is one quoted word already. Keys come from the server's
`rubato.*` names; a character outside `[A-Za-z0-9._-]` becomes `_` (today only `,`, space and `=`),
so a key can never break the `k=v,k=v` format. Values keep today's replacement of `,` and space.

### 2.4 `claude_command`

It is the developer's own `local.yml`, may carry arguments (`claude --model …`), and is written
into the script as is. That is documented on `CompanionConfig::claude_command` and in the
script's comment. The companion never takes it from the server.

### 2.5 Map ids

`rubato_protocol::map` gains:

- `MAP_ID_PATTERN = "^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$"` and `is_map_id`, the same rule
  as the draft's `valid_id`: lower-case ASCII letters, digits and hyphens, 1 to 64
  characters, no hyphen at either end;
- `MapError::BadId { what, id }`, pushed by `SystemMap::validate` for each node and seam id
  that fails;
- `#[schemars(pattern(MAP_ID_PATTERN))]` on `MapNode.id` and `Seam.id`, so
  `schema/system-map.schema.json` and every protocol schema that embeds `SystemMap` gain the
  pattern. plan-check and the skill validate maps with that schema through ajv, so they apply
  the rule with no code change.

The draft's `valid_id` becomes a call to `is_map_id`. Seam `from`, `to` and `owner` and flow
ends get no pattern: they must equal a node id, which `validate` already checks. Scenario ids are
Q3.

### 2.6 Existing maps

Every map in the repo passes the rule (checked with a script over the YAML; the same check runs
as a test, §3):

| File | Ids | Outside the rule |
| --- | --- | --- |
| `docs/design/rubato-map.yml` | 25 | 0 |
| `e2e/fixtures/team-map.yml` | 8 | 0 |
| `web/e2e/fixtures/shop-map.yml` | 5 | 0 |
| `schema/fixtures/bad-map.yml` | 3 | 0 |
| `schema/fixtures/maps/*.yml` (4 files) | 25 | 0 |

A team map outside this repo with an id outside the rule fails to load on the next reload: the
server keeps serving the map it has (`map/load.rs:141-161`, the cached map from `warm` is not
re-validated), counts `rubato.map.load_failures` and logs "system map invalid: id … is not a map
id". plan-check fails the map's next PR on the schema. That is Q2.

## 3. Tests

| Test | Fails today because |
| --- | --- |
| `ids.rs`: `is_branch_for` accepts `issue/7-restock`, `issue/7-öppna-kartan`; refuses `issue/9-x` for #7, `issue/7-../x`, `issue/7-a/b`, `issue/7-x.lock`, `main`, `issue/7` | the function does not exist |
| `map.rs`: `validate` reports `BadId` for `A`, `a_b`, `-a`, `a'b`, 65 characters, on a node and on a seam; accepts `a`, `a-b`, 64 characters | `validate` has no id rule |
| `map.rs`: every map in the repo (the eight files above) validates; prints how many maps and ids it checked | must pass; fails on purpose with `schema/fixtures/maps/bad-ids.yml` |
| `schema_snapshot.rs`: snapshots regenerated with `UPDATE_SCHEMAS=1` | the pattern is new |
| plan-check Vitest: `bad-ids.yml` fails `loadMapText` on the schema | the schema has no pattern |
| `tmux_ide.rs`: a stub tmux-ide records its argv; `worktree_create` passes `-- issue/7-x` last, `adopt` passes `-- <name>` | no `--` today |
| `script.rs`: a node id `a'$(touch${IFS}<dir>/p)'` in `otel_resource`; the script passes `sh -n`; run with `claude_command` stubbed to print `OTEL_RESOURCE_ATTRIBUTES`, it prints the id literally and `<dir>/p` does not exist | passes on main (regression guard for #156) |
| `script.rs`: a key `a b,c=d'e` is written as `a_b_c_d_e` | only `,`, space and `=` are replaced today |
| Gherkin `launch-branch.feature`: a launch for issue 7 on branch `issue/9-other` fails with "is not issue/7-", and leaves no worktree or tmux session | `check_ref` accepts that branch today |
| Gherkin `launch-branch.feature`: a slug `../../escape` fails the same way and leaves nothing | passes on main (`check_ref` refuses `..`); guards the worktree path |
| Gherkin `launch-branch.feature`: a node id with `'$(touch${IFS}pwned)'` reaches the stand-in claude literally, and no `pwned` file exists in the worktree | passes on main; regression guard on the real launch path |

## 4. What this does not do

- No new `RefusalReason`: that is a protocol enum change on `companion-server` for a case no
  Rubato server produces. The launch stays `Failed` (A4).
- No change to `slugify` unless Q1 is answered (b).
- No percent-encoding of resource attribute values (the OTel spec allows it); `,` and space
  are still replaced, as Claude Code's exporter reads them today.
