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

The server is a **GitHub App** (ADR 0022). One registration does three things: it logs people
in, it is the identity the server reads and writes GitHub with, and it decides who is on the
team: a person gets a session only if their GitHub account can access the repositories the App
is installed on.

1. Register a GitHub App at <https://github.com/settings/apps/new> (an existing OAuth app
   cannot be converted; this is a new registration with its own client id).
   - Homepage URL: the URL the server will be reachable at (for a first run on one machine,
     `http://127.0.0.1:8080`). Callback URL: that URL plus `/auth/github/callback`.
   - Webhook: off. Rubato polls.
   - Repository permissions: *Contents: read and write*, *Pull requests: read and write*,
     *Issues: read and write*, *Checks: read*, *Metadata: read*. Nothing else. Issues: write
     is for "New issue here…" (ADR 0025); without it Rubato still runs and the form says why
     it cannot create.
   - An App registered before #71 has *Issues: read*. Change it to *read and write*; GitHub
     then asks the installation's owner to approve the new permission. Rubato reads the
     installation's permissions on every full poll pass (every five minutes), so no restart is
     needed.
   - Where it can be installed: only on this account. One installation covers one account,
     so all the team's repositories sit under it.
2. On the App's page: note the **App ID** and the **Client ID**, generate a **client secret**,
   and generate a **private key** (a `.pem` file downloads).
3. Install the App on the account, on every repository the map names and the one that holds
   the map. The **installation id** is the number at the end of the installation's settings URL
   (`…/settings/installations/<id>`). A repository you leave out is shown as a finding on
   the map, not as an empty repository.

Without the App settings the server still runs the old way, for a first look: an OAuth app for
login and `RUBATO_GITHUB_TOKEN` (read-only) for reads. It then warns at start that login is
open to any GitHub account, and it cannot propose map changes.

## 3. Run the server

```sh
export RUBATO_BIND=127.0.0.1:8080
export RUBATO_PUBLIC_URL=http://127.0.0.1:8080
export RUBATO_DATABASE_URL="sqlite://$HOME/.local/state/rubato/server.db?mode=rwc"
export RUBATO_SESSION_SECRET="$(openssl rand -hex 32)"
export GITHUB_CLIENT_ID=...            # the App's client id, from step 2
export GITHUB_CLIENT_SECRET=...
export RUBATO_GITHUB_APP_ID=...
export RUBATO_GITHUB_APP_PRIVATE_KEY_FILE=/path/to/the-app.private-key.pem
export RUBATO_GITHUB_APP_INSTALLATION_ID=...   # from step 3
# Optional: lets people accept a plan from Rubato, posted as themselves (ADR 0030). Keep it
# secret and stable: rotating it makes every stored token unreadable (everyone logs in again).
# The App must have "Expire user authorization tokens" on (the default).
export RUBATO_TOKEN_KEY="$(openssl rand -hex 32)"
export RUBATO_MAP_REPO=m0x6A/rubato
export RUBATO_MAP_PATH=docs/design/rubato-map.yml
export RUBATO_MAP_REF=main
export RUBATO_WEB_DIST="$PWD/web/dist"
mkdir -p "$HOME/.local/state/rubato"
./target/release/rubato-server
```

The log should show `acting as a GitHub App installation`, `map loaded` with the git sha of
`main` and `rubato-server listening`. If the key, the client id and the installation do not
belong together the server stops at start with `github app check failed` and says which.
Open <http://127.0.0.1:8080> and log in with GitHub. The map appears, and the Issues tab lists
every open issue of the map's repositories under "Not pinned".

A session lasts as long as its team check is at most seven days old; after that, or after the
first start as an App, the browser sends you through login once more. The private key can
also be given inline as `RUBATO_GITHUB_APP_PRIVATE_KEY` (newlines as `\n`).

Editing the map: "Edit map" opens the team's shared draft. "Propose map change…" opens one pull
request on the map repository from a branch `map/draft-<time>`; that needs the App's Contents
and Pull requests permissions set to read and write. On a personal token, or with
`RUBATO_MAP_FILE`, the server cannot propose and "Copy YAML" gives the proposed file instead.

For a GitHub other than github.com (a GitHub Enterprise Server), set `RUBATO_GITHUB_API_URL`
to its REST API root and `RUBATO_GITHUB_WEB_URL` to its web root. The browser tests use the
same two settings to run the whole App login against a stand-in (`web/e2e/github-standin.ts`).

Other knobs: `RUBATO_GITHUB_POLL_SECS` (default 60), `RUBATO_SESSION_STALE_SECS` (120),
`RUBATO_COMPANION_SILENT_SECS` (600), `RUBATO_DEFAULT_SPEND_CAP_USD` (25),
`RUBATO_DEFAULT_ATTEMPT_LIMIT` (3), `OTEL_EXPORTER_OTLP_ENDPOINT` to export traces and
metrics. For a run without GitHub at all, `RUBATO_MAP_FILE` reads the map from a local file
and `RUBATO_LOCAL_REPOS=owner/name=/path/to/repo.git` reads branches and plans from local
bare repositories, with `RUBATO_DEV_ISSUES=<file.json>` giving them open issues;
`RUBATO_DEV_USER=<login>` skips OAuth and must never be set where others
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

### Placing issues from other tools

A tool that opens issues for you (the voice-note flow, a script) can place each issue on the map
by ending its body with one line (ADR 0026):

```text
Map: node <id>
Map: seam <id>
```

The id must be a node or seam id of the map; `GET /api/map` lists them. The rules:

- The line holds nothing else, starts with `Map:` exactly, and is not inside a code fence. When
  there are several, the last one counts.
- It places the issue only when its author is on the team: GitHub's `author_association` is
  `OWNER`, `MEMBER` or `COLLABORATOR`. The repository is public, so a line from anyone else is
  shown in *Not pinned* as a suggestion that a person confirms with **Pin here**.
- It pins an issue that is not pinned yet, once. When someone unpins the issue, the same line
  does not pin it again; change the line or pin it by hand.
- An id the map does not have leaves the issue in *Not pinned*; the "Map lines" panel counts it
  as `unknown_id`.

Issues without the line get a suggested place too, from ids, paths and words of the map that
appear in their title and body.

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
endpoint under `otel` in `workspace.yml` so launched sessions export too. The rules in
`observability/alerts/rubato.rules.yml` cover the "never silent" list from `AGENTS.md`;
`promtool test rules observability/alerts/tests/rubato.test.yml` fires each on purpose.

### Publishing to Grafana

A merge to `main` that changes `observability/` publishes the dashboard and the alert rules
(`.github/workflows/publish-observability.yml`). The dashboard goes into the folder "Rubato".
The rules are sent in their Prometheus form to Grafana's import route and become
Grafana-managed rules in the same folder, evaluated against the metrics datasource. The job
publishes only from `main`, also when started by hand.

The workflow reads three repository variables and one secret:

| Name | Kind | Value |
| --- | --- | --- |
| `GRAFANA_URL` | variable | the stack's address, such as `https://<stack>.grafana.net` (not the OTLP gateway) |
| `GRAFANA_PROM_DS` | variable | the name of the metrics datasource |
| `GRAFANA_LOKI_DS` | variable | the name of the logs datasource |
| `GRAFANA_API_TOKEN` | secret | a service account token that may read datasources and write folders, dashboards and alert rules |

Set the variable `GRAFANA_ALERTS_PAUSED` to `true` to import the alert rules paused: they are
on the stack but do not evaluate or fire. Remove it and run the workflow again to make them
active.

By hand, with the same four in the environment:

```
pnpm observability:publish --dry-run       # resolves datasources and the folder, writes nothing
pnpm observability:publish                 # writes, reads back and compares
pnpm observability:publish --verify-live   # metrics the files query that the stack has never stored
```

Sending a rule group replaces it, so a rule removed from the file leaves the stack. A whole
group or dashboard removed from the repository stays in Grafana until someone deletes it there.
Until a server exports to the stack, `ServerPollSilent` fires unless the rules are paused: it
is the rule for "no poll tick seen".
