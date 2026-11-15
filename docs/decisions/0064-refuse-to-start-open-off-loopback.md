# 0064. Refuse to start open to anyone unless the server is local or the operator opts in

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/169#issuecomment-6892513103 (self); drafted by agent (Claude Code, issue m0x6A/rubato#169), proposed
- **Seam:** web-server
- **Issue:** m0x6A/rubato#169

## Context

ADR 0022 made access to the GitHub App's installation the team, and said that a server without
the App configuration "behaves as before". Before meant two open doors. With an OAuth app and no
GitHub App, any GitHub account can log in and read the overview, pin, queue, launch and pair
(`auth/handlers.rs:161-199` checks the team only as an App), and its web sessions never expire on
the server (`auth/store.rs:41-58`). With `RUBATO_DEV_USER` set, every client is that user and may
name any other login in the `x-rubato-dev-user` header (`auth/current_user.rs:32-39`). The server
starts in either mode with at most a warning (`main.rs:19-21`), so an operator can expose it by
setting a public URL and nothing more. Plan: `docs/plans/169.md`, design
`docs/plans/169-refuse-open-start/design.md`.

## Decision

A server is **local** when it binds a loopback address (`RUBATO_BIND`, 127.0.0.0/8 or ::1) and its
public URL (`RUBATO_PUBLIC_URL`) names a loopback host (`localhost`, 127.0.0.0/8 or ::1). A server
that is not local refuses to start, with the reason on stderr, when `RUBATO_DEV_USER` is set
(there is no opt-in for it), or when OAuth is configured (`GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET`) without the GitHub
App and `RUBATO_OPEN_LOGIN=1` is not set. `RUBATO_OPEN_LOGIN` accepts `1` or unset; any other value
is a start-up error. A local server in either mode, and an opted-in server, start as before and
say at start, as a warning, that login is open. A web session counts for at most 30 days from its
creation, the cookie's own lifetime, in every mode; as an App the 7-day team record of 0022 still
applies on top.

## Alternatives considered

- **Warn only, as today**: the operator who sets a public URL is told in a log line nobody reads,
  and the server is open.
- **Check the public URL only (the issue's wording)**: a server bound to 0.0.0.0 with a loopback
  public URL is reachable from the network and would pass.
- **An opt-in for the dev user too**: the header lets any client be anyone, including a team
  member; there is no deployment where that is wanted off loopback.
- **Refuse OAuth-only outright**: removes the "first look" path of the quickstart for someone
  who has not registered the App yet; the opt-in keeps it, named and loud.

## Consequences

An open server off loopback is a choice the operator writes down (`RUBATO_OPEN_LOGIN=1`), not an
accident. The dev user works only on the developer's own machine; the e2e harnesses already run
on 127.0.0.1. A container image binds 0.0.0.0, so a container in OAuth-only mode needs the opt-in
and a container with the dev user does not start. Sessions without the App now end after 30 days
on the server as well as in the browser. A refusal is a process exit with the reason, covered by
the `web-server` O-row of plan #169 (the start-up error text, the container health check, and
`rubato.web.rejected{reason=session_expired}` for the session limit).

## Supersedes

0022, in part: the sentence "A server without the App configuration (a personal token, or none,
or the development user) behaves as before". The rest of 0022 stands. On acceptance, 0022's status
line gains "; open login without the App superseded by 0064".
