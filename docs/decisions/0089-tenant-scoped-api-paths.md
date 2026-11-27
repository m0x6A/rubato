# 0089. Serve tenant routes under `/api/t/{tenant}`; map routes under `/api/t/{tenant}/w/{workspace}`

- **Status:** proposed
- **Date:** 2026-11-27
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #345)
- **Seam:** web-server
- **Issue:** m0x6A/rubato#345

## Context

ADR 0049 scopes map routes under `/api/w/{ws}`, keeps team routes unprefixed and lets an
unprefixed map route mean `default`. With tenants (ADR 0087) every route that returns or changes a
tenant's data must name the tenant, and nothing may fall back to a default one. Design:
`docs/plans/345-tenants/design.md` §4.1–§4.2.

## Decision

Routes that read or change a tenant's data are served under `/api/t/{tenant}/…`, and map-scoped
ones under `/api/t/{tenant}/w/{ws}/…`. The browser socket is `/ws/web?t={tenant}`. Person and
server routes stay unprefixed: `/api/me` (which lists the person's tenants and roles),
`/auth/*`, `/api/pair/*`, `/api/pairings`, `/ws/companion`, `/api/graph/snapshot`,
`/api/health`, `/webhooks/github`. Operator routes are `/api/admin/…`. A tenant the caller is not
a member of, a workspace not in the tenant, and a resource of another tenant all answer 404, never
403. The `/api/w/{ws}` and unprefixed tenant forms answer 404, and no workspace
id is required to exist in a tenant. The router is built from one table of routes, which the
cross-tenant test also reads. Every `WebEvent` names its tenant,
and the server sends a socket only its tenant's events. The web keeps the tenant and workspace in
its URL, `/t/{tenant}/w/{ws}`.

## Alternatives considered

- **Tenant from a header or the session**: invisible in links and logs; two tabs could not show
  two tenants.
- **Keep unprefixed routes for a default tenant**: a fallback is where a scoping bug hides.
- **Globally unique workspace ids, tenant derived**: shorter paths, but leaks which ids exist in
  other tenants and makes the scope implicit.

## Consequences

The web's `scoped()` and every fetch change in one pull request; the companion and the skill use
only unprefixed person and server routes and are unchanged. `WebEvent` gains `tenant`, a protocol
schema change. Covered by the `web-server` O-row (`rubato.web.rejected{reason=unknown_tenant}`).

## Supersedes

0049, and in part 0044 (a `default` workspace is required and serves unprefixed routes).
