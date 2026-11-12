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
| Image source | local, built here and sent with `docker save | ssh apps docker load` (unregistry is not installed here) |
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
| Environment (names only; values live in `/opt/stacks/rubato/.env` on the host) | `RUBATO_PUBLIC_URL`, `RUBATO_SESSION_SECRET` (secret), `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` (secret), `RUBATO_GITHUB_TOKEN` (secret); `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_HEADERS` (secret; Grafana Cloud OTLP gateway, set 2026-11-12); optional `RUBATO_MAP_REPO`, `RUBATO_MAP_PATH`, `RUBATO_MAP_REF` |
| Persistent data | `./data` → `/data` (`rubato.db`, owned by uid 1000) |

## Deploys

| Date | Tag | Commit | Previous tag | Result |
| --- | --- | --- | --- | --- |
| 2026-11-10 | `sha-df0e692` | `df0e692` | — (first deploy) | healthy, 200 on `/api/health` from the LAN, map loaded, logs clean |
| 2026-11-12 | `sha-e4488fa` | `e4488fa` | `sha-df0e692` | healthy, 200 on `/api/health` from the LAN, logs clean (one warning: no GitHub App configured); migrations 0002–0005 applied, database copied first to `data/rubato.db.before-e4488fa`; metrics reach Grafana Cloud (17 of 27 queried names stored after two minutes) |
