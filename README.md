# Rubato

**Each developer orchestrates their own agents. Rubato keeps the whole picture and keeps teammates, human and agent, aligned.**

Rubato is a planning-time tool for a team that owns several interconnected repositories. It shows one system map above the repos, pins issues to the parts of the system they touch, shows which agent is working on which issue and where that work lands on the map, and brings each agent's plan back into the shared picture. It never runs an agent itself and never reaches into a developer's machine: the companion on each machine connects outbound and acts only for its own user.

It is a lab to verify a set of theses about agent orchestration, not a product. v0 is complete and on `main`; see [the v0 plan](docs/plans/v0.md) and the [quickstart](docs/QUICKSTART.md).

## How it fits together

| Part | Where | Role |
| --- | --- | --- |
| `rubato-server` | one instance per team | holds the map and the issue pins, takes signals from companions and GitHub, derives progress per issue, fans changes out to browsers, serves the web view |
| `rubato-companion` | one per developer machine | pairs with one GitHub login, connects outbound, launches Claude Code on an issue in a git worktree inside the developer's own tmux, forwards Claude Code hook events and tmux-ide session state, stops a session at its spend cap |
| `web` | served by the server | the map with issues, sessions and plans painted on nodes and seams; pin an issue; start a session in your own tmux; talk to your own session; read every revision of a plan draft and approve it; the plan's room |
| `skills/rubato` | installed on each developer machine | the Claude Code skill a session uses to read the map and the ADRs, write its plan at the agreed path, record decisions in it, and hand the approved plan off to Git in one commit |
| `actions/plan-check` | GitHub Action | fails a PR whose plan names an id the map does not declare or touches a seam without an O-row |
| `rubato-protocol` | shared crate | the contract between all of the above: serde types, exported JSON Schemas, the progress derivation, the `rubato.*` telemetry names |

The things we stitch rather than build: [tmux-ide](https://tmux-ide.com) runs and discovers sessions, Claude Code is the only agent and its hooks are the event source, GitHub holds code, issues, PRs, checks and the plan files.

## The progress model

One state per issue, derived from signals, never stored:

Planned (pinned) → Started (launched) → Designing (working, no plan yet) → Designed (plan on the branch) → Building (working after the plan) → In review (PR open, checks green) → Done (merged), with Blocked (hook says so, or a cap was hit) and Unknown (no trustworthy signal) cutting across. Identity comes from the branch: pane cwd → worktree → `issue/<n>-<slug>` → issue. A session with no recent hook event renders *unknown*, never *working*.

## Repository layout

```
crates/rubato-protocol     shared types, schema export, otel names
crates/rubato-companion    the per-developer daemon (Gherkin in features/)
crates/rubato-server       the shared server (sqlx migrations in migrations/)
web/                       React + React Flow; src/protocol.ts is generated from schema/protocol
skills/rubato              the Claude Code skill and its scripts
actions/plan-check         the plan check (Node) and its Action
schema/                    JSON Schemas and the parser fixtures shared by TypeScript and Rust
observability/             Grafana dashboard, alert rules with promtool tests, the structural check
e2e/                       the end-to-end run: two repos, three issues, real binaries, a Claude Code stand-in
docs/design/rubato-map.yml Rubato's own system map
docs/plans/                plans, one per task
docs/decisions/            ADRs
AGENTS.md                  how we work; read it first
```

## Building and testing

Rust stable (edition 2024), Node 22.6 or newer, pnpm, tmux 3.7 or newer and tmux-ide for the companion's suite.

```sh
cargo build --workspace
cargo test --workspace            # includes the companion's Gherkin suite on a private tmux server
pnpm install && pnpm -r test      # plan check, skill, web
pnpm plan-check --map docs/design/rubato-map.yml --plans "docs/plans/*.md"
pnpm observability:check
pnpm e2e                          # two repos, three issues through the flow, no GitHub, no tokens spent
```

CI runs all of the above plus Playwright against the server and `promtool test rules` on the alert rules.

### Several agents on one machine

Each agent session in its own worktree runs its own `cargo build` and `rust-analyzer`, so a few at once can use up the machine's memory. The workspace's `[profile.dev]` already keeps line tables for our own crates (backtraces and `tracing` still show file and line) and drops debug info for dependencies. Release builds are unchanged. The rest is per machine and not committed:

- **Share compiled dependencies** across worktrees and `cargo clean` with [sccache](https://github.com/mozilla/sccache): `cargo install sccache --locked`, then `export RUSTC_WRAPPER=sccache` in your shell profile. It caches dependencies only. The workspace's own crates build incrementally, and sccache skips those.
- **Cap parallel jobs** on a machine that runs several agents, in `~/.cargo/config.toml`:
  ```toml
  [build]
  jobs = 4
  ```
- **Keep the desktop responsive** (Linux, optional): start the agents' tmux in a memory-limited scope, `systemd-run --user --scope -p MemoryHigh=22G -p MemoryMax=26G -p CPUWeight=50 tmux`. Size the limits to the machine (these are for 32 GB). `/proc/pressure/memory` shows whether it helps.
- **Cranelift for dev builds** (optional, local only): it needs nightly, and the repo stays on stable, so keep it out of committed config.

## Status and limits of v0

- Plans are drafted in Rubato, mirrored from the session's worktree, and travel as Markdown files on the issue branch once approved; the server reads them from GitHub at the branch head.
- The join between issues and map nodes lives in the server's SQLite. GitHub Projects v2 is the later candidate.
- Spend is summed from the session transcript with prices you configure; nothing is guessed.
- GitHub itself has only been exercised against recorded responses in tests. The quickstart is the first real run.
