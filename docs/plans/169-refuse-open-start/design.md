# #169: refuse to start open to anyone off loopback (design)

Plan: [../169.md](../169.md) · ADR 0064 (*proposed*, partly supersedes 0022) · Issue: m0x6A/rubato#169

## The two open doors, as the code stands (ed20a6c)

| Mode | What lets anyone in | Where |
| --- | --- | --- |
| OAuth without the App | the callback creates a session for any GitHub account; the team check and its freshness gate run only when `github_app` is set | `auth/handlers.rs:161-199`, `auth/current_user.rs:49` |
| OAuth without the App | a session row never expires on the server; only the 30-day cookie ends it in the browser | `auth/store.rs:41-58`, `auth/handlers.rs:234` |
| `RUBATO_DEV_USER` | every request is the dev login, and `x-rubato-dev-user` names any other | `auth/current_user.rs:32-39` |
| both | the server starts with one `warn!` (dev user) or one `warn!` only on the token branch (OAuth-only) | `main.rs:19-21`, `main.rs:66-68` |

## D1. Local means bind and public URL are both loopback

`Config::from_env` already knows `bind` (a `SocketAddr`) and `public_url`. A server is local when
`bind.ip().is_loopback()` **and** the public URL's host is `localhost` or a loopback IP (parsed with
the `url` crate, already a dependency). Either one alone is not enough:

- bind 127.0.0.1, public URL `https://rubato.example` — a reverse proxy publishes it;
- bind 0.0.0.0, public URL `http://127.0.0.1:8080` — anyone on the network reaches the port.

The issue says "public URL"; checking both is stricter. Raised as Q1.

## D2. What refuses and what warns

`fn exposure(local, dev_user, oauth_only, open_login) -> Result<Vec<String>>` in
`kernel/config.rs`, pure, called at the end of `from_env`. Its warnings join `Config::warnings`,
which `main` logs once the subscriber exists.

| Local | `RUBATO_DEV_USER` | OAuth without App | `RUBATO_OPEN_LOGIN` | Result |
| --- | --- | --- | --- | --- |
| yes | set | any | any | start, warn "every request is that user" |
| no | set | any | any | **refuse**: "RUBATO_DEV_USER is set and the server is not local (bind …, public URL …)" |
| yes | unset | yes | unset | start, warn "login is open to any GitHub account" |
| no | unset | yes | unset | **refuse**: "OAuth is configured without the GitHub App … set RUBATO_OPEN_LOGIN=1 to run open" |
| no | unset | yes | `1` | start, warn "login is open to any GitHub account (RUBATO_OPEN_LOGIN=1)" |
| any | any | any | other than `1` | **refuse**: "RUBATO_OPEN_LOGIN must be 1 or unset" |
| any | unset | no | `1` | start, warn "RUBATO_OPEN_LOGIN=1 has no effect" |

"OAuth without the App" is `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET` set and the App settings absent. Without both of them and without the
dev user the server is closed: `/auth/github/login` answers 501 (`auth/handlers.rs:64-70`) and nobody gets a session.

The existing `main.rs` warnings move into `exposure`, so there is one wording per mode. Refusal
happens before telemetry starts (as the other `from_env` errors do), so its only signal is the
process exit with the message on stderr; that is what an operator sees in `docker logs` or the
Dockge stack and what makes the container health check fail.

## D3. Sessions end on the server after 30 days

`store::web_session` also returns `created_at`. `CurrentUser` refuses a session older than
`SESSION_MAX_AGE_SECS` (30 days, the cookie's `Max-Age`) in every mode, with
`app.reject(path, "session_expired", login)`, so it lands on the existing `rubato.web.rejected`
counter and the "Web writes rejected" panel. As an App the 7-day team record still applies on
top. A loopback OAuth-only user logs in again after 30 days, as the browser already makes them.

## Who relies on the open modes today

| Where | Mode | Local | After this change |
| --- | --- | --- | --- |
| `web/e2e/run.sh:60` server 1 | dev user | 127.0.0.1 bind and URL | starts |
| `web/e2e/run.sh:89` server 2 | App | 127.0.0.1 | starts (not open) |
| `e2e/run.ts:128-130` | dev user | 127.0.0.1 bind and URL | starts |
| `crates/rubato-server/tests/*` | `Config::for_tests` | not read from env | unaffected |
| `docs/QUICKSTART.md` §3 | App on 127.0.0.1 | yes | starts; the OAuth-only paragraph gains the opt-in |
| `Dockerfile` | binds 0.0.0.0 | no | App: starts; OAuth-only needs `RUBATO_OPEN_LOGIN=1`; dev user refused |
| `.claude/profiles/dockge.md` (the running instance) | App configured | no | starts (not open) |
| `.github/workflows/ci.yml` | runs the e2e scripts above | yes | starts |

So no running configuration known to the repo is open off loopback; the issue's "needs
validation" is answered for the instance recorded in the Dockge profile.

## Not done here

- The dev-user header is still honoured on loopback, where it is what the browser tests need.
- No new metric for "started open": the start-up warning is the signal, and an opted-in server is
  a choice already written in its environment.
