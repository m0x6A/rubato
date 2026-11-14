# 0046. Let a `Map:` line name its workspace

- **Status:** accepted
- **Date:** 2026-11-14
- **Decided by:** m0x6A, accepted 2026-11-14 by approval comment https://github.com/m0x6A/rubato/issues/89#issuecomment-6876029281 (self); drafted by agent (Claude Code, issue #89 design session)
- **Seam:** server-github
- **Issue:** m0x6A/rubato#89

## Context

ADR 0026 places an issue with `Map: node <id>` or `Map: seam <id>` in its body. In a repo named by
two workspaces (ADR 0045) a person may want the issue in one only. Design D4.

## Decision

The line is `Map: node|seam <id> [in <workspace-id>]`. Unqualified, it places the issue in every
workspace of its repo whose map has that id with that kind. Qualified, only in the named workspace;
an unknown workspace id places nothing and counts as `unknown_workspace` on
`rubato.backlog.map_lines`. Rubato writes the qualifier on issues it creates (ADR 0025) only when
the repo belongs to more than one workspace. A server without this ADR ignores the four-word form,
since its parser takes exactly two words, so nothing is placed by mistake.

## Alternatives considered

- **Always qualify**: noise for the common single-workspace repo, and breaks every existing line.
- **A second line `Workspace: x`**: two lines to keep together, and an orphaned one places wrongly.

## Consequences

Existing lines keep working. Covered by the server-github O-row.

## Supersedes

0026's grammar (its rules on placement, fences and the last line otherwise stand); amends 0025.
