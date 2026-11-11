# 0022. Register Rubato as a GitHub App; access to its installation decides who is on the team

- **Status:** proposed
- **Date:** 2026-11-10
- **Decided by:** agent (Claude Code, issue #48 design session), from Mattias Jonsson's answers in that session ("give the app access through the app registration"; team gate "App access"); proposed for him to accept
- **Seam:** server-github, web-server
- **Issue:** m0x6A/rubato#48

## Context

The server logs people in through an OAuth app and reads GitHub with a personal token (`docs/QUICKSTART.md`). The OAuth callback creates a session for any GitHub account, and sessions do not expire on the server. That was tolerable while a stranger who found the URL could at most pin an issue. Issue #48 adds team-wide writes (layout, draft), shows every open issue title of the map's repos (ADR 0019) and has the server open pull requests on the map repo (ADR 0018). The design review raised both the open login and the write credential as blockers. Design: `docs/plans/48-interactive-map/design.md` (D12).

## Decision

Rubato is registered as a GitHub App, installed on the team's repositories with Contents and Pull requests read and write, and Issues, Checks and Metadata read. The server is configured with the App id, its private key and the installation id, and makes every GitHub call as that installation. People still log in through the App's OAuth flow. A person gets a session only if their account can access the installation, checked at login; the time of that check is stored on the session, and every request and websocket upgrade requires it to be present and at most 7 days old, otherwise the person logs in again. All the team's repositories are under one GitHub account, since one installation covers one account. At start the server checks the App settings (key, client id, installation) and refuses to start with the reason when they do not fit together. A server without the App configuration (a personal token, or none, or the development user) behaves as before; it cannot propose map changes, because only an installation can write.

## Alternatives considered

- **A list of logins in the environment**: independent of GitHub, but someone edits it for every colleague.
- **A personal token with write access**: one person's identity and rate limit behind everything the server does.
- **Checking only at login**: sessions created before the gate, or before someone lost access, would keep full access.
- **GitHub organisation or team membership**: needs an organisation and a wider scope; repo access through the installation says the same thing for one team.

## Consequences

Whoever has access to the repos the App is installed on is on the team, and nobody maintains a list. Pull requests from Rubato come from the App's identity and name the people in their body. Losing repo access removes a person within 7 days, or at once if their sessions are deleted. Everyone logs in once more after the upgrade. Setting up a server now includes registering and installing the App, which a human does; an existing OAuth app cannot be converted, so the client id and secret change. Reads now see only what the installation covers: a repo outside it is reported as a finding, not treated as empty (`rubato.github.repos_uncovered`). `octocrab` gains its `jwt` feature. A refused or looping login is covered by the `web-server` O-row (`rubato.web.rejected{reason=not_on_team|team_record_stale}`, the `login refused` log line).

## Supersedes

None. It narrows who gets the session that 0004 and 0011 rely on, and replaces the personal token as the server's GitHub identity without changing what 0008, 0010 or 0014 read.
