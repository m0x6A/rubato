# 0030. Rubato acts on GitHub as the person through the App's user-to-server tokens

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #80), from Mattias Jonsson's request in the planning session; a human accepts
- **Seam:** server-github
- **Issue:** m0x6A/rubato#80

## Context

Mattias asked that a plan can be accepted in Rubato. Acceptance is an `Approved:` comment by a
person (ADR 0027); a comment the App posts is authored by a Bot and is, by that ADR, ignored, and
it should be: the audit trail must name who approved. Today login asks GitHub for `read:user` and
throws the token away. Merging pull requests from Rubato, the follow-up issue, needs the same thing.

## Decision

Login already runs through the GitHub App's client (ADR 0022). The server keeps each person's
user-to-server access token and refresh token in SQLite (table `user_tokens`, owned by the `auth`
slice), encrypted with AES-256-GCM under `RUBATO_TOKEN_KEY` from config, only when it runs as an App
and the App issues expiring user tokens (the response carries `refresh_token` and `expires_in`).
Refresh is single-flight per login, and a rejected refresh deletes the row only if it still holds
the refresh token that was sent (App refresh tokens rotate). Tokens are deleted on logout and when
the person's last web session expires or the ADR 0022 team re-check fails. Without the key, in
OAuth-only mode, or with non-expiring tokens, nothing is stored and every person stays read-only. A
token is used only by a server slice acting for that person on a request they made in the browser;
in v0 the only such write is the approval comment (`approve` slice). The token never leaves the
server: it is not sent to the web, the companion or any agent. GitHub records the comment as the
person's, via the App, and enforces the person's own permissions.

## Alternatives considered

- **Post as the App and name the person in the text**: a Bot comment is ignored by ADR 0027, and the audit would say the App approved.
- **A classic OAuth app with `repo` scope**: a broad, long-lived token; the App's user tokens are scoped to the App's permissions and expire.
- **No write; link to GitHub with the comment prefilled**: works, but is not "accept in Rubato".

## Consequences

The server now holds credentials that act as people. A leaked database plus key acts as every
person with a stored token, within the App's permissions: access tokens last 8 h, but the stored
refresh tokens last about six months, so the exposure lasts until each person's token is revoked
(revoking the App's authorization in GitHub, or rotating the key, which makes every row unreadable).
ADR 0025 rejected stored user tokens for issue creation because the App was enough there; for an
approval it is not, since a Bot's `Approved:` is ignored (ADR 0027) and the audit must name the
person. Issue creation stays with the App.
Merging (follow-up issue) needs the App's contents and pull-request write permissions and its own
ADR amendment. O-row: server-github (span `rubato.plan.approve`, log `token refresh failed`).

## Supersedes

None. ADR 0025 (the App creates issues as itself) stands.
