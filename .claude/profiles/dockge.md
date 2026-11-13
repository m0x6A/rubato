# Dockge profile

What the `dockge-deploy` skill needs to know about where this repository's app runs.
Every fact carries the date it was last verified. Secret values never go here, only
variable names.

## Host

| Fact | Value | Verified |
| --- | --- | --- |
| SSH host (alias in `~/.ssh/config`) | `apps` | 2026-11-10 |
| Stacks folder | `/opt/stacks` (`DOCKGE_STACKS_DIR`) | 2026-11-10 |
| Dockge UI | `http://192.168.50.49:5001` | 2026-11-10 |
| Host name apps are reached on | `192.168.50.49` (`apps` resolves only to an IPv6 link-local address from the dev machine) | 2026-11-10 |
| Helper CLI | `apps` (`/usr/local/bin/apps`); its `deploy` pulls, so not used for this stack | 2026-11-10 |
| Stacks in git | no | 2026-11-10 |
| Logged in to ghcr.io on the host | no | 2026-11-10 |
| Port range for apps | 8081–8199 | 2026-11-10 |
| Stacks folder backed up | no: `./data` (the SQLite database) has no copy | 2026-11-10 |
| Docker on the host | linux/amd64, 1.9 GiB memory, SSH user has uid 1000 | 2026-11-10 |

## Build

| Fact | Value |
| --- | --- |
| Image source | local, built here and sent with `docker save local/rubato:<tag> \| gzip -1 \| ssh apps 'gunzip \| docker load'`. unregistry is not installed: `docker pussh --help` exits 0 anyway, so do not test for it that way (2026-11-12) |
| Image | `local/rubato` |
| Platform | `linux/amd64` |

Dockge's **Update** button pulls and must not be used for this stack; redeploy with the skill.

## App

| Fact | Value |
| --- | --- |
| Stack name | `rubato` |
| Container port | `8080` (`RUBATO_BIND=0.0.0.0:8080`, set in the image) |
| Host port | `8081` |
| Health check | `GET /api/health` → 200 |
| Environment (names only; values live in `/opt/stacks/rubato/.env` on the host) | `RUBATO_PUBLIC_URL`, `RUBATO_SESSION_SECRET` (secret), `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` (secret); GitHub App: `RUBATO_GITHUB_APP_ID`, `RUBATO_GITHUB_APP_INSTALLATION_ID`, `RUBATO_GITHUB_APP_PRIVATE_KEY_FILE=/data/github-app.pem` (verified 2026-11-12); `RUBATO_TOKEN_KEY` (secret, 64 hex, generated on the host 2026-11-12; ADR 0030); `RUBATO_GITHUB_TOKEN` (secret, ignored as an App); `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS` (secret; Grafana Cloud OTLP gateway, set 2026-11-12); optional `RUBATO_MAP_REPO`, `RUBATO_MAP_PATH`, `RUBATO_MAP_REF` |
| Persistent data | `./data` → `/data` (`rubato.db`, owned by uid 1000) |

## Deploys

| Date | Tag | Commit | Previous tag | Result |
| --- | --- | --- | --- | --- |
| 2026-11-10 | `sha-df0e692` | `df0e692` | — (first deploy) | healthy, 200 on `/api/health` from the LAN, map loaded, logs clean |
| 2026-11-12 | `sha-e4488fa` | `e4488fa` | `sha-df0e692` | healthy, 200 on `/api/health` from the LAN, logs clean (one warning: no GitHub App configured); migrations 0002–0005 applied, database copied first to `data/rubato.db.before-e4488fa`; metrics reach Grafana Cloud (17 of 27 queried names stored after two minutes) |
| 2026-11-12 | `sha-2f8a446` | `2f8a446` | `sha-e4488fa` | healthy (deployed outside this profile; row added from the host's image list); running as a GitHub App |
| 2026-11-12 | `sha-1603f90` | `1603f90` | `sha-2f8a446` | healthy, 200 on `/api/health` from the LAN, logs clean; migrations 0006–0010 applied (`user_tokens`, `plan_approvals`), database copied first to `data/rubato.db.before-1603f90`; `RUBATO_TOKEN_KEY` replaced (was 40 chars, the server needs 64 hex). About 3 minutes down: the first `up` ran before the image was on the host |
| 2026-11-13 | `sha-9db533e` | `9db533e` | `sha-1603f90` | healthy, 200 on `/api/health` from the LAN, logs clean, map loaded (11 nodes, 11 seams); restyle #81 served; migration 0011 (`repo_pulls`, additive) applied, database copied first to `data/rubato.db.before-9db533e` |
