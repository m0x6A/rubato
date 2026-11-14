# 0047. Keep layout, draft, proposal, pins and map cache per workspace

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, issue #89 design session); a human accepts
- **Seam:** web-server, server-system-map
- **Issue:** m0x6A/rubato#89

## Context

ADR 0016 keeps one shared layout, ADR 0018 one shared draft and one proposal, ADR 0008 pins
keyed by issue and target. With several maps, the same node id in two maps would share a position,
a draft would not know which map file it rewrites, and a pin made while looking at one application
would appear in the other. Design D5, D7.

## Decision

`map_layout`, `map_cache`, `map_proposal`, `map_draft_ops`, `pins`, `map_line_pins`,
`issue_creations`, `launches` and the pin suggestions carry a `workspace_id`; existing rows become
`default` in migration 0013. Every query on these tables binds the workspace;
the rebuilt tables have no column default, so a missed bind fails. Each workspace has one layout,
one draft and at most one proposal, written to that workspace's map repo on a branch named
`map/draft-<workspace>-<time>`. A launch records the workspace it was started from, and a map
version mismatch is judged against that workspace's map. A pin made in the web goes into the workspace being viewed;
other workspaces of the repo show "pinned in <name>" on the issue. Rows whose workspace is no longer
configured are kept, logged at start and counted as `rubato.workspace.orphan_rows`. GitHub facts
keyed by repo and number (plans, pull requests, decisions, open issues, approvals, spend) stay
unscoped and shared.

## Alternatives considered

- **Pins shared across workspaces**: a pin is a statement about one map's id.
- **One table per workspace**: unbounded schema, and migrations per workspace.

## Consequences

Two people can edit two maps' drafts at once. Covered by the web-server O-row
(`rubato.workspace` on route spans) and `rubato.map.proposals{rubato.workspace}`.

## Supersedes

0016 and 0018 (each holds per workspace); amends 0008's pin key.
