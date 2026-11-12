# 0025. Rubato creates issues as the GitHub App, pinned where they were started

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** proposed by agent (Claude Code, claude-opus-5-5, issue #71); awaiting Mattias Jonsson
- **Seam:** server-github, web-server
- **Issue:** m0x6A/rubato#71

## Context

#71 asks that issues can be created from inside Rubato and follow the right pattern. Today nothing
creates issues: the port has no issue write and the App's Issues permission is read only
(ADR 0022). Map proposals already write as the App (ADR 0018). The user's OAuth token has scope
`read:user` and is not kept.

## Decision

The App's installation permissions gain Issues: write. The server creates the issue through the
installation token, with a body it renders from structured fields under #71's headings (Summary,
What the change does, How we'll know it works as checkboxes, What it must not break, Context, Open
questions), a line naming the creator in backticks (no notification) and a `Map:` line
(ADR 0026). The issue is pinned at creation to the node or seam it was started from, with the
creator as `pinned_by`; the pin is recorded as already applied for that `Map:` line, so a later
unpin sticks. Creation is offered only when the installation's permissions, read as the App from
`GET /app/installations/{id}` at start and on every full poll pass, include `issues: write`; a 403
on create clears the capability until the next read. Otherwise the form says why.

A request carries a `request_id`, and creation is at most once per id. The POST is sent with
retries off (the client otherwise retries hung calls, and an issue POST is not idempotent). The body
carries a `Rubato-Request: <id>` line. A row stays `opening` until the POST answers or the next
open-issues listing shows an issue with that line, which moves it to `created`; a row still opening
after 10 minutes becomes `failed`. A second submit of an id that is `opening` answers "still
opening" and sends nothing.

## Alternatives considered

- **Create as the person**: needs `repo` scope and stored user tokens on the server.
- **Hand off to GitHub's new-issue page**: no permission change, but no pin at creation and the pattern is not enforced.

## Consequences

The issue's author on GitHub is the App; the body names the person. Installations must re-approve
the permission; Rubato notices within one full pass, without a restart. A creation that times out
may exist on GitHub without a pin; the listing finds it by its `Rubato-Request:` line and pins it.
Rows stuck in `opening` are a gauge (`rubato.issues.opening`). O-rows: server-github (`rubato.issues.created{outcome}`, span
`rubato.github.call{endpoint=create_issue}`), web-server (`rubato.web.rejected` reasons).

## Supersedes

The permission list in 0022 only (Issues: read → read and write). The rest of 0022 stands.
