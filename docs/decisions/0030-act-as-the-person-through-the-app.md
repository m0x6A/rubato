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

Login becomes the GitHub App's user authorization. The server keeps each person's user-to-server
access token and refresh token in SQLite (table `user_tokens`, owned by the `auth` slice),
encrypted with AES-256-GCM under `RUBATO_TOKEN_KEY` from config, refreshes it on expiry, and deletes
it on logout. Without the key the server stores no tokens and every person stays read-only. A
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
person who logged in, within the App's permissions, until the tokens expire (8 h) or are revoked.
Merging (follow-up issue) needs the App's contents and pull-request write permissions and its own
ADR amendment. O-row: server-github (span `rubato.plan.approve`, log `token refresh failed`).

## Supersedes

None. ADR 0025 (the App creates issues as itself) stands.
