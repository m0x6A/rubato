# Quickstart: the first real run, on this repository

This walks through running Rubato for real, with GitHub and a live Claude Code session, using
the rubato repository as both the map repo and the one connected repo. Everything up to the
last step is free; the last step spends tokens under a cap.

What you need: a machine with tmux 3.7 or newer, `git`, Claude Code, Node 22.6 or newer, pnpm,
Rust stable, `curl`, and a GitHub account that can read this repository. tmux-ide is no longer
needed (retired by ADR 0070): the companion runs tmux and git itself.

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
login and `RUBATO_GITHUB_TOKEN` (read-only) for reads. Login is then open to any GitHub
account, and the server cannot propose map changes. It starts that way only when it is local
(`RUBATO_BIND` a loopback address and `RUBATO_PUBLIC_URL` a loopback host); anywhere else it
refuses to start unless you set `RUBATO_OPEN_LOGIN=1`, and it warns at start either way (ADR 0064).

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
# The cutover (ADR 0054): from this time on plans are approved in Rubato, and an Approved:
# comment posted at or after it does not count. Leave it unset until every companion and skill
# is upgraded (the "Companions by capability" panel shows session_input, draft_mirror and
# plan_handoff for each); while unset, plans are approved by comment as before.
# export RUBATO_COMMENT_APPROVALS_UNTIL=2026-11-20T10:00:00Z
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

A session lasts as long as its team check is at most seven days old, and never more than 30
days (in every mode); after that, or after the first start as an App, the browser sends you through login once more. The private key can
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
metrics. `RUBATO_COMPANION_SILENT_SECS` is the one companion stale threshold N: the overview
carries it to the companion lanes' freshness bars, and the gauge `rubato_companion_silent_secs`
carries it to the `CompanionSilent` alert (which falls back to 600 without that series). A value
below 60 is raised to 60 with a warning at startup. For a run without GitHub at all, `RUBATO_MAP_FILE` reads the map from a local file
and `RUBATO_LOCAL_REPOS=owner/name=/path/to/repo.git` reads branches and plans from local
bare repositories, with `RUBATO_DEV_ISSUES=<file.json>` giving them open issues;
`RUBATO_DEV_USER=<login>` skips OAuth: every request is that login, and the
`x-rubato-dev-user` header names any other. The server refuses to start with it unless it is
local (a loopback `RUBATO_BIND` and `RUBATO_PUBLIC_URL`; ADR 0064).

**The context graph (ADR 0084), off by default.** `RUBATO_CONTEXT_GRAPH=1` makes the server
build a graph of every repository the maps name, at its default-branch head: files, decision
records, feature scenarios and slices, and the symbols, imports and references of Rust,
TypeScript and JavaScript sources (names and one signature line each, never a file's contents),
beside each workspace's map and pins and the team's pinned issues, plans and open pull requests. It regenerates every `RUBATO_GRAPH_REGEN_SECS` (300–86400,
default 900): one GitHub call per repository per tick, and a tarball only when the head moved.
The graph lives in its own SQLite file beside the main database (`rubato-graph.db`;
`RUBATO_GRAPH_DATABASE_URL` moves it), is never backed up, and is rebuilt if deleted. With the
flag on the server needs the GitHub App unless it is local, and refuses to start beside
`RUBATO_OPEN_LOGIN` (ADR 0086). Nothing reads the graph yet: companions and the MCP tools arrive
with issues #329 to #332.

Observed traffic on the map's seams (ADR 0039) comes from one of two sources, never both;
setting both is a startup error. Either way the server fetches every `RUBATO_TRAFFIC_FETCH_SECS`
(15–300, default 60). The `RUBATO_TRAFFIC_PROMETHEUS_*` and `RUBATO_DEFAULT_*` variables below
only seed the database, once; after the first start each workspace's source and launch defaults
are set in Configure workspace (see "Workspace settings live in the database").

**Production: the service graph (ADR 0040).** The server reads the OpenTelemetry servicegraph
connector's `traces_service_graph_request_total` and `…_request_failed_total` through any
Prometheus-compatible query API (`POST /api/v1/query`). Rubato never receives the traces
themselves.

| Variable | Default | Meaning |
| --- | --- | --- |
| `RUBATO_TRAFFIC_PROMETHEUS_URL` | unset | base URL of the query API; unset means this source is off |
| `RUBATO_TRAFFIC_PROMETHEUS_HEADERS` | unset | `Name=value,…` sent with every query, for example `Authorization=Bearer …`; never logged |
| `RUBATO_TRAFFIC_WINDOW_SECS` | 3600 | how far back the counts reach, 60–86 400 |
| `RUBATO_TRAFFIC_PROMETHEUS_TIMEOUT_SECS` | 10 | per query, at most half the fetch interval |
| `RUBATO_TRAFFIC_PROMETHEUS_METRIC_PREFIX` | `traces_service_graph_` | for a generator that names them differently |
| `RUBATO_TRAFFIC_PROMETHEUS_NAMESPACE_LABELS` | `client_service_namespace,server_service_namespace` | the two namespace labels, client first |
| `RUBATO_TRAFFIC_PROMETHEUS_NAMESPACES` | unset: every namespace | the service namespaces to read, comma-separated, at most 20; a call is kept when its client or its server is in one of them, so a stack shared with other applications does not show theirs |

The team's side of the setup:

1. **Every service emits its identity.** Each one sets `service.namespace` and `service.name`
   (`OTEL_RESOURCE_ATTRIBUTES` / `OTEL_SERVICE_NAME`). Rubato sets no namespace for a service it
   observes. Its own parts (`rubato-server`, the companions, the sessions they launch, the skill
   scripts and the plan check) emit `service.namespace=rubato` unless their environment sets
   another (ADR 0080).
2. **The connector carries the namespace.** It drops it unless asked. In the Collector or Alloy,
   set it as a dimension:

   ```yaml
   connectors:
     servicegraph:
       dimensions: [service.namespace]
   ```

   Tempo's metrics-generator takes the same as `service_graphs.dimensions`.

   **The pairing wait must cover the web.** An edge needs the client span and the server span
   of one call to reach the generator within its wait: the connector's `store.ttl` (2 s by
   default) or Tempo's `service_graphs.wait` (10 s). The web posts its spans within 1 s through
   the server's relay, and the server exports every 5 s (ADR 0082), so set at least 10 s:

   ```yaml
   connectors:
     servicegraph:
       dimensions: [service.namespace]
       store:
         ttl: 10s
   ```

   Grafana Cloud's generator waits 10 s. The dashboard panel "HTTP trace context" shows the
   server's side: `linked` is a request under the web's span.
3. **Calls to parts that emit nothing get `peer.service`.** For GitHub or a file, the caller sets
   `peer.service` on its client span. The connector then draws a virtual node, which has no
   namespace. The skill, plan-check and the server name the plan file and the map this way
   (`peer.service=plan-file`, `system-map`), so those parts carry `service` without a namespace.
4. **The web's own spans go through the server (ADR 0082).** With `OTEL_EXPORTER_OTLP_ENDPOINT`
   set on the server, the web posts its client spans to `POST /api/telemetry/traces`, and the
   server forwards them, rewritten to `service.name=web`, with its own OTLP headers. Without an
   endpoint the route answers 404 and the web stops sending.
5. **plan-check in CI.** Set the repository secrets `RUBATO_OTLP_ENDPOINT` and
   `RUBATO_OTLP_HEADERS` to the same values as the server's; the plan-check step reads them. A
   fork's pull request gets no secrets and emits nothing.

A failing query API makes the state `failing` (log `traffic source failing`). A malformed answer
is rejected and the previous report kept. A source set up wrong (a bad URL, window or label)
is failing from the start, and the log line says which variable. None of them is ever read as
quiet.

**Which part a service is (ADR 0059).** A part says which service it is in the map, exactly as
that service emits itself. Change it through the map draft like any other field:

```yaml
nodes:
  - id: rubato-server
    kind: context
    service: { namespace: rubato, name: rubato-server }
  - id: github
    kind: external
    service: { name: github }   # a virtual node from callers' peer.service: no namespace
```

How an edge end is read:

- **It matches a part** only when namespace and name are both equal. A missing namespace equals
  only a missing one. There is no match by id and none by name alone. A part without `service`
  is never matched, and never shown as quiet.
- **It becomes a ghost** when it emits a namespace but no part names it. The Traffic lens draws it
  in a band right of the map, with its calls. Under *Map versus territory* it can be taken into
  the draft, filled in from the telemetry, or dismissed for the workspace (ADR 0060).
- **It is only counted** when it emits no namespace and no part matches it, as "with no namespace
  and no part to match".

**Development: a file.** `RUBATO_DEV_TRAFFIC=<file.json>` reads a traffic report from a file.

- **Names.** Each edge names a client and a server by their OTel `service.name`, and may add
  `client_namespace` and `server_namespace`.
- **Matching.** An edge without namespaces matches parts that declare `service: { name: … }`
  with no namespace. A name with neither a namespace nor such a part is counted as unmapped:

```json
{
  "window_secs": 3600,
  "edges": [
    { "client": "web", "server": "rubato-server", "calls": 1200, "errors": 12 },
    { "client": "rubato-server", "server": "github", "calls": 300 }
  ]
}
```

`observed_at` may be left out; it is then the file's modification time, so a file nobody
rewrites goes stale after three fetch intervals and the overview says `stale`, not quiet. A
report with more than 2 000 edges, a window outside 60 s to 24 h or a name over 200 bytes is
rejected whole (log line `traffic report rejected`) and the previous one kept. Unset, the
overview says `not_configured`.

## 4. Install the skill on your machine

```sh
node --experimental-strip-types skills/rubato/scripts/install.ts
```

This puts the skill at `~/.claude/skills/rubato` with the plan-check code and the schemas
vendored, and runs `npm install` there. Claude Code picks it up in every project.

## 5. Describe your machine

Create `~/.config/rubato/local.yml` from `skills/rubato/local.example.yml`. (It was called
`workspace.yml`; that name is still read, with a warning. Rename it only after upgrading the
companion, since an older companion reads only `workspace.yml`.) A *workspace* is now one
application's map on the server (ADR 0044); list each map you plan against under `maps:`.

```yaml
server_url: http://127.0.0.1:8080
maps:
  - repo: m0x6A/rubato
    path: docs/design/rubato-map.yml
    local: /home/you/develop/rubato      # your clone; the skill reads the map from here
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

### Several workspaces on one server

A server holds one map by default (`RUBATO_MAP_*`). To hold several, list them in a file and
set `RUBATO_WORKSPACES_FILE` instead (ADR 0044). One entry must be `default`: it keeps the rows
from before and serves the plain `/api/...` routes.

```yaml
workspaces:
  - id: default
    map_repo: m0x6A/rubato
    map_path: docs/design/rubato-map.yml
  - id: shop
    name: Shop
    map_repo: acme/shop-map
    map_path: system-map.yml
```

Migration 0013 is not additive: copy the database file before upgrading, so the previous
version can be restored.

### Workspace settings live in the database (ADR 0075)

What describes a workspace is set at runtime, from **Configure workspace** in the header: its
name, its map ref, its traffic source and its launch defaults. A team member can also create a
workspace there (an id, a name, and a map that their own GitHub login can read, in a map whose
repos they can read too) and delete any but `default`. Application settings (the bind address,
the database, the GitHub App, the token key, the poll and stale thresholds) stay in the
environment and are not shown there.

- **The environment seeds the database once.** The first start after the upgrade writes one row
  per entry of `RUBATO_WORKSPACES_FILE` (or `default` from `RUBATO_MAP_*`), and copies the
  `RUBATO_TRAFFIC_*` and `RUBATO_DEFAULT_*` values that are set into every one of them. A
  running deploy keeps working with no config change. After that the database owns these
  settings: changing the file or those variables changes nothing.
- **Every start warns while they are still set**: `workspace setting still set outside the
  database`, once per variable or file entry, with `differs` saying whether its value is another
  than the database holds. Remove them when convenient. `map_file`, `RUBATO_MAP_FILE` and
  `RUBATO_DEV_TRAFFIC` are not seeded: they are paths on the server's disk, read at every start.
- **Traffic headers are stored sealed** with `RUBATO_TOKEN_KEY`. A start that would seed
  `RUBATO_TRAFFIC_PROMETHEUS_HEADERS` without the key refuses and says so: set the key, or unset
  the headers. Rotating the key makes the stored headers unreadable; the source then shows as
  failing with `secrets_unsealable` until someone sets them again in Configure workspace.
- **A traffic URL is checked.** It must be http or https with no user or password. Unless the
  server is local, loopback, link-local (cloud metadata), IPv6 unique-local and `100.64.0.0/10`
  addresses are refused; the RFC 1918 ranges are allowed.
- **A server open to any GitHub account** (OAuth without the App, `RUBATO_OPEN_LOGIN=1`) refuses
  every settings write.
- **Each change is logged** as `workspace config changed` with who and which keys, never the
  values (dashboard panel "Workspace config changes").

**Back up the database before this upgrade.** Migration 0027 adds the table, and an older binary
refuses a database that has it. Rolling back means restoring that backup; workspaces created at
runtime, and settings changed since, are then lost.

## 6. Pair and run the companion

In a terminal that is **not** inside tmux:

```sh
./target/release/rubato-companion pair --server http://127.0.0.1:8080
```

It prints a code such as `KMCN-C2MH`. In the web view, open the *Companions* tab, paste the
code and approve. The companion stores its pairing token in
`~/.local/state/rubato/companion.json` with mode 0600 and exits.

The server URL must be `https://`, or `http://` on loopback (`localhost`, `127.0.0.1`, `::1`)
as above (ADR 0067). For a server that serves plain http on another host, such as a homelab
LAN address, pair with `--insecure`:

```sh
./target/release/rubato-companion pair --insecure --server http://192.168.1.20:8080
```

The pairing token and launch commands then travel unencrypted. The choice is stored in
`companion.json` (`allow_plain_http: true`), so `run` needs no flag, and the companion logs a
warning naming the URL at pairing and at every start. Pair again without `--insecure` to undo it.
Then:

```sh
./target/release/rubato-companion run
```

The *Companions* tab shows your machine as connected. Leave it running; it reconnects by itself.

**What leaves your machine** (ADR 0052), which `pair` also prints:
- the sessions the companion sees: repo, branch, issue, status and spend;
- heartbeats and launch results;
- each hook's event name and metadata: session, cwd, tool name;
- for sessions started from Rubato only, your prompts and the agent's replies, cut to
  `Limits.session_log_max_chars`. The whole team reads them in the session's log;
- for sessions started from Rubato only, the plan the session writes: the plan file at its
  path, the design note it links and the ADRs it proposes, up to `Limits.draft_max_bytes`, as a
  draft the team reads in Rubato (ADR 0053). Nothing else in the worktree is read.
- for sessions started from Rubato only, the agent's answers to turns in the plan's room, when
  you called it in: typed into the session while planning, or from a read-only fork while
  building, with the fork's cost. Everyone who reads the room reads them (ADR 0057).

Tool input and output stay on your machine, and so do the prompts and replies of sessions you
start yourself. The companion cuts every hook payload to an allowlist before it leaves, and the
server keeps only the log text, never a hook payload.

## 7. Put an issue through the flow

1. Open a small issue on `m0x6A/rubato`, for example a docs fix. Note its number *n*.
2. In the web view, *Pin an issue*: `m0x6A/rubato#n` to the node it touches (`web`,
   `rubato-server`, ...). The issue appears as **Planned**.
3. Select it and press **Start session in my tmux**. The button only exists while a companion
   of yours is connected; it never targets anyone else's machine.

What happens next, and what to watch:

- The companion creates the worktree `../rubato-worktrees/issue/n-<slug>` with `git worktree` and a
  tmux session named `rubato@issue-n-<slug>` running Claude Code with `--session-id`, the hook
  configuration injected with `--settings`, and `rubato.repo`, `rubato.issue`,
  `rubato.branch` and `rubato.node` set as OTel resource attributes. Attach with
  `tmux attach -t rubato@issue-n-<slug>` to watch or take over.
- The issue moves to **Started**, then **Designing** as the first hook events arrive.
- The session, following the skill, writes `docs/plans/n.md` and checks it.
  - **Before the cutover** it pushes the plan and posts a `Review:` comment, and a person
    approves it with an `Approved:` comment.
  - **After the cutover** it leaves the draft uncommitted in the worktree. At each pause the
    companion mirrors the plan, its design note and its proposed ADRs to Rubato, and the issue
    shows the draft with every revision and what changed since you last looked. Talk to your
    session from the issue panel; the agent replies with `Review:` and `Summary:` there. Press
    **Approve** on the newest revision. The companion types the handoff, the plan reaches the
    branch in one commit with the approval record, and the issue gets one link comment from
    you.

  Once the plan is on the branch, or a draft has settled, the issue is **Designed** and the
  plan's affected parts paint the map. Two plans on one seam show as a finding on that seam,
  and as a notice in both plans' rooms.
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
  is refused and reported; a missing repo in `local.yml` fails the launch with the reason, and a
  tmux below 3.7 (`companion.tmux_min_version`) is refused as `missing_dependency`. A directory
  already at the worktree path that git does not list fails the launch with the path; move it
  away, the companion never deletes it.
- The issue stays **Started**: no hook event has arrived. `rubato-companion status` prints the
  hook endpoint and the header file that holds its token; `curl -H @<header file>` it from the
  worktree to prove the path (curl 7.55 or later; never put the token itself on a command line,
  ADR 0063), and check the session's `--settings` file was passed.
- Spend stays empty: the companion reads a transcript only under `~/.claude/projects` (or
  `$CLAUDE_CONFIG_DIR/projects`), as a regular file of at most 256 MiB, and logs
  `rubato.transcript.refused` with the reason otherwise.
- The issue shows **Unknown**: the trust check. The last signal is older than the stale
  threshold, or the session was started by hand and sends no hooks. `rubato-companion status`
  lists each repo and worktree as opted in, stale, or with the command that opts it in:
  `rubato-companion render-hooks --into <dir>/.claude/settings.local.json` (ADR 0078). A hook
  event fixes it.
- The plan never shows: the server polls GitHub every `RUBATO_GITHUB_POLL_SECS`; check the
  server log for `poll failed for issue` and the token's scopes.

## Observability

Point `OTEL_EXPORTER_OTLP_ENDPOINT` at your collector for both binaries and set the same
endpoint under `otel` in `local.yml` so launched sessions export too. The rules in
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
