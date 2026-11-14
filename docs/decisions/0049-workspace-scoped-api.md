# 0049. Scope map routes under `/api/w/{workspace}`; team routes stay unprefixed

- **Status:** proposed
- **Date:** 2026-11-14
- **Decided by:** agent (Claude Code, issue #89 design session); a human accepts
- **Seam:** web-server
- **Issue:** m0x6A/rubato#89

## Context

The browser reads one overview and writes pins, layout, drafts, issues, launches and approvals
against one map. With workspaces (ADR 0044) every map-scoped request must say which one, and a
write must never land in a workspace by default by accident. Design D7.

## Decision

Map-scoped routes (overview, map, map reload, draft, layout, pins, issues, plans/approve, launch)
are also served under `/api/w/{ws}/…`; the unprefixed form means the workspace with id `default`
(ADR 0044) and stays for one release so a cached web bundle keeps working. The web uses the
prefixed form for every map-scoped call once the overview has told it its workspace. Team-scoped routes stay unprefixed: me, auth, pair,
pairings, queue, `/ws/companion`, `/ws/web`, health. `GET /api/workspaces` lists
`WorkspaceInfo {id, name, map_repo, version, loaded_at}`. An unknown workspace answers 404 with
reason `unknown_workspace`. The overview carries `workspace` and `workspaces`. `WebEvent::Changed`
gains an optional `workspace`, set only for changes that belong to one map (layout, draft, map
load); pins, issues, approvals and polls leave it absent, since they show in other workspaces too. The browser keeps its choice in `?w=<id>` and the
last choice in local storage, not in the path, because deep paths fall back to `index.html`. A
stored or linked id the server answers `unknown_workspace` for is cleared, and the web falls back
to `default`.

## Alternatives considered

- **A query parameter on every route**: easy to forget on a POST, and a silent default on a write.
- **A workspace field in every payload**: changes every protocol type and leaves GETs unsolved.
- **A header**: invisible in logs and links.

## Consequences

Easier: a link names a workspace. Harder: two route forms until the unprefixed one is retired by a
later ADR. Covered by the web-server O-row (`rubato.web.rejected{rubato.reason=unknown_workspace}`).

## Supersedes

none
