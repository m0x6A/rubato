# 0026. A `Map:` line in the issue body places the issue on the map

- **Status:** accepted
- **Date:** 2026-11-12
- **Decided by:** Mattias Jonsson, accepted 2026-11-12 (drafted by agent, Claude Code, claude-opus-5-5, issue #71)
- **Seam:** server-github
- **Issue:** m0x6A/rubato#71

## Context

Issues written from voice notes by a tool outside this repo land in "Not pinned" and a person has to
find their place by hand. #71 asks that every such issue shows where in the application it fits.
The open-issues listing (ADR 0019) already returns each issue's body, so reading it costs no calls.

## Decision

A line `Map: node <id>` or `Map: seam <id>` in an issue body, outside code fences, the last one
winning, places the issue when its author is trusted: association OWNER, MEMBER or COLLABORATOR,
read from the same listing, or the App itself for issues Rubato created (ADR 0025). The server pins
an unpinned issue when the id exists with that kind, at most once per (issue, target), so a person's
unpin sticks. The repo is public, so a line from anyone else is not trusted: that issue gets the
named place as its suggestion, and the outcome is counted as `untrusted_author`. An unpinned issue
with no usable line gets a scored suggestion (id tokens, part paths, names
and description words in its title and body) that a person confirms with one click through the
existing pin route. In-app creation (ADR 0025) and the external voice tool write the same line;
`docs/QUICKSTART.md` documents it for other tools.

## Alternatives considered

- **Suggestions only**: every voice issue would still need a person to place it.
- **Move the voice flow into Rubato**: the tool works elsewhere already; one line is a smaller contract.

## Consequences

Only team members (and Rubato itself) place issues automatically, and only on existing ids, so a
stranger cannot add issues to the tracked set that the poll pays for each tick. A 4 KB body
excerpt is stored server side for scoring and never sent to browsers. O-row: server-github
(`rubato.backlog.map_lines{outcome}`, log `backlog placement` with counts).

## Supersedes

None.
