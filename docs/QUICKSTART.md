# Quickstart: the first real run, on this repository

This walks through running Rubato for real, with GitHub and a live Claude Code session, using
the rubato repository as both the map repo and the one connected repo. Everything up to the
last step is free; the last step spends tokens under a cap.

What you need: a machine with tmux 3.7 or newer, [tmux-ide](https://tmux-ide.com) 2.9 or
newer, Claude Code, Node 22.6 or newer, pnpm, Rust stable, `git`, `curl`, and a GitHub
account that can read this repository.

## 1. Build

```sh
git clone https://github.com/m0x6A/rubato && cd rubato
pnpm install
cd web && pnpm build && cd ..
cargo build --release --workspace
```

The binaries are `target/release/rubato-server` and `target/release/rubato-companion`.

## 2. GitHub credentials for the server

The server needs two things from GitHub: an OAuth app to log people in, and a token of its own
to read repositories.

1. Create an OAuth app at <https://github.com/settings/developers>. Homepage: the URL the
   server will be reachable at (for a first run on one machine, `http://127.0.0.1:8080`).
   Authorization callback: that URL plus `/auth/github/callback`. Note the client id and
   generate a client secret.
2. Create a token the server uses for reads: a fine-grained token with *Contents: read*,
   *Issues: read*, *Pull requests: read* and *Metadata: read* on the repositories you connect
   (here: `m0x6A/rubato`), or a classic token with `repo` scope.

## 3. Run the server

```sh
export RUBATO_BIND=127.0.0.1:8080
export RUBATO_PUBLIC_URL=http://127.0.0.1:8080
export RUBATO_DATABASE_URL="sqlite://$HOME/.local/state/rubato/server.db?mode=rwc"
export RUBATO_SESSION_SECRET="$(openssl rand -hex 32)"
export GITHUB_CLIENT_ID=...            # from step 2
export GITHUB_CLIENT_SECRET=...
export RUBATO_GITHUB_TOKEN=...         # from step 2
export RUBATO_MAP_REPO=m0x6A/rubato
export RUBATO_MAP_PATH=docs/design/rubato-map.yml
export RUBATO_MAP_REF=main
export RUBATO_WEB_DIST="$PWD/web/dist"
mkdir -p "$HOME/.local/state/rubato"
./target/release/rubato-server
```

The log should show `map loaded` with the git sha of `main` and `rubato-server listening`.
Open <http://127.0.0.1:8080> and log in with GitHub. The map appears with no issues yet.

Other knobs: `RUBATO_GITHUB_POLL_SECS` (default 60), `RUBATO_SESSION_STALE_SECS` (120),
`RUBATO_COMPANION_SILENT_SECS` (600), `RUBATO_DEFAULT_SPEND_CAP_USD` (25),
`RUBATO_DEFAULT_ATTEMPT_LIMIT` (3), `OTEL_EXPORTER_OTLP_ENDPOINT` to export traces and
metrics. For a run without GitHub at all, `RUBATO_MAP_FILE` reads the map from a local file
and `RUBATO_LOCAL_REPOS=owner/name=/path/to/repo.git` reads branches and plans from local
bare repositories; `RUBATO_DEV_USER=<login>` skips OAuth and must never be set where others
can reach the server.

## 4. Install the skill on your machine

```sh
node --experimental-strip-types skills/rubato/scripts/install.ts
```

This puts the skill at `~/.claude/skills/rubato` with the plan-check code and the schemas
vendored, and runs `npm install` there. Claude Code picks it up in every project.

## 5. Describe your workspace

Create `~/.config/rubato/workspace.yml` from `skills/rubato/workspace.example.yml`:

```yaml
server_url: http://127.0.0.1:8080
map:
  repo: m0x6A/rubato
  path: docs/design/rubato-map.yml
  local: /home/you/develop/rubato        # your clone; the skill reads the map from here
  decisions: docs/decisions
repos:
  - name: m0x6A/rubato
    local: /home/you/develop/rubato
otel:                                      # optional; drop the block to export nothing
  endpoint: http://127.0.0.1:4318
  protocol: http/protobuf
companion:
  pricing:                                 # USD per million tokens, by model id prefix; see the model's price list
    "claude-":
      input: 0
      output: 0
      cache_read: 0
      cache_write: 0
```

Fill the prices from your plan's current price list; with zeros the companion still counts
tokens but reports no cost. The companion only launches into repositories listed under
`repos`, and the clone must have `origin` pointing at GitHub.

## 6. Pair and run the companion

In a terminal that is **not** inside tmux:

```sh
./target/release/rubato-companion pair --server http://127.0.0.1:8080
```

It prints a code such as `KMCN-C2MH`. In the web view, open the *Companions* tab, paste the
code and approve. The companion stores its pairing token in
`~/.local/state/rubato/companion.json` with mode 0600 and exits. Then:

```sh
./target/release/rubato-companion run
```

The *Companions* tab shows your machine as connected. Leave it running; it reconnects by itself.

## 7. Put an issue through the flow

1. Open a small issue on `m0x6A/rubato`, for example a docs fix. Note its number *n*.
2. In the web view, *Pin an issue*: `m0x6A/rubato#n` to the node it touches (`web`,
   `rubato-server`, ...). The issue appears as **Planned**.
3. Select it and press **Start session in my tmux**. The button only exists while a companion
   of yours is connected; it never targets anyone else's machine.

What happens next, and what to watch:

- The companion creates the worktree `../rubato-worktrees/issue/n-<slug>` with tmux-ide and a
  tmux session named `rubato@issue-n-<slug>` running Claude Code with `--session-id`, the hook
  configuration injected with `--settings`, and `rubato.repo`, `rubato.issue`,
  `rubato.branch` and `rubato.node` set as OTel resource attributes. Attach with
  `tmux attach -t rubato@issue-n-<slug>` to watch or take over.
- The issue moves to **Started**, then **Designing** as the first hook events arrive.
- The session, following the skill, writes `docs/plans/n.md`, checks it and pushes it. On the
  next poll the issue is **Designed** and the plan's affected parts paint the map. Two plans on
  one seam show as a finding on that seam.
- Spend appears on the session row as the transcript grows; at the cap the companion kills the
  session and the issue is **Blocked**.
- Open a PR from the branch: **In review** when checks are green, **Done** when merged. The
  plan check in CI fails a plan that names an unknown id or lacks an O-row.

## 8. If it does not move

- Nothing happens after *Start session*: read the companion log. A launch for the wrong user
  is refused and reported; a missing repo in `workspace.yml` or a tmux-ide below 2.9 fails
  the launch with the reason.
- The issue stays **Started**: no hook event has arrived. `rubato-companion status` prints the
  hook endpoint; `curl` it from the worktree with the token to prove the path, and check the
  session's `--settings` file was passed.
- The issue shows **Unknown**: the trust check. The last signal is older than the stale
  threshold, or the only state comes from tmux-ide's screen reading. A hook event fixes it.
- The plan never shows: the server polls GitHub every `RUBATO_GITHUB_POLL_SECS`; check the
  server log for `poll failed for issue` and the token's scopes.

## Observability

Point `OTEL_EXPORTER_OTLP_ENDPOINT` at your collector for both binaries and set the same
endpoint under `otel` in `workspace.yml` so launched sessions export too. Import
`observability/dashboards/rubato-overview.json` into Grafana and load
`observability/alerts/rubato.rules.yml` into Prometheus or Mimir. The five rules cover the
"never silent" list from `AGENTS.md`; `promtool test rules observability/alerts/tests/rubato.test.yml`
fires each on purpose.
