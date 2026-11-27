# 0087. A tenant owns GitHub App installations, workspaces and members; every tenant-owned row carries its tenant

- **Status:** proposed
- **Date:** 2026-11-27
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #345), drafting; that membership is by invite, that the invite alone is the gate, that a person may be in several tenants, that the tenant is a column in one database, that no tenant needs a `default` workspace, that a detached installation's rows are tombstoned until an operator purges them, and that graph sources are filtered before a cap raised to 256 are Mattias Jonsson's choices, 2026-11-27
- **Seam:** server-github, web-server, companion-server
- **Issue:** m0x6A/rubato#345

## Context

One server is one team today: whoever can access the one configured installation (ADR 0022), and
every member sees every workspace (ADR 0044). A demo on the internet has to hold several teams on
one server without any of them seeing another's issues, plans, sessions, rooms or code structure.
GitHub already partitions what the App can read into installations, one per account (a person or
an organisation). Design: `docs/plans/345-tenants/design.md` §3–§6.

## Decision

A **tenant** is a named set of GitHub App installations, workspaces and members. An installation
belongs to at most one tenant, so a repository belongs to at most one tenant (`tenant_repos`,
refreshed from each installation's repository list and its `installation_repositories`
webhooks), and every repository an installation covers may be named by the tenant's maps.

A workspace belongs to one tenant, and its id is unique inside that tenant. No tenant needs a
`default` workspace, and nothing falls back to one. Every row of a workspace, a repository fact
or a launch carries `tenant_id NOT NULL` with no default, in its key and in every conflict
target, written when the row is written. A row whose repository no tenant covers is not written,
so an installation nobody attached is never read. Every GitHub client is asked for by tenant and
repository, and is refused for a repository not live in that tenant. A map that names such a
repository shows it as uncovered, at creation and at every load. A detached or deleted
installation's rows are kept but hidden and no longer read; an operator purges them, and
attaching the installation to another tenant is refused until then.

A person (users, sessions, tokens, pairings, a companion's session snapshots) is global; what
they see is the union of their memberships, one tenant at a time. A person joins a tenant only by
an invite naming their GitHub user id, and the invite is the whole gate: the server reads the
tenant's repositories as the installation whether or not the invitee's own GitHub account can,
and through Rubato the invitee also writes as the App where Rubato does (it creates issues, and
pushes a branch, a file and a pull request for a map draft). Acting on GitHub as the person
still uses their own token. Who issues invites is ADR 0088.

A new map source passes when its map repository and every repository the map names are live in
the tenant; the member's own token no longer vets it. The browser socket, the overview,
background housekeeping (seam notices, findings, gauges) and companion intake each work inside
one tenant. A companion receives, and may fetch, only the context graph sources of its owner's
tenants, filtered before a cap of 256. The existing deployment becomes one tenant on first start.

## Alternatives considered

- **GitHub org membership as the gate**: a demo visitor is not in the org; and a tenant spanning
  several orgs and personal accounts would need a rule per account.
- **Invite and GitHub access both** (ADR 0022 kept as a second gate): safer for private
  repositories, but a visitor would need access granted on GitHub too; the admin decides instead.
- **A database file per tenant**: stronger isolation, but every migration, the poll loop and the
  graph would run per file; a column with checked rules is enough for a lab.
- **One server per tenant**: no shared operator page, one App registration and host per team.
- **Purge at detach, or re-tag rows on attach**: the first loses history to a slip; the second
  hands one tenant's rooms and drafts to another.

## Consequences

Isolation rests on a parsed-SQL check that every query on a tenant-owned table is scoped, a sweep
that calls every route as a member of another tenant (with every flag on) and expects 404, and
tests for the paths no route reaches (design §11). Every store function gains a scope argument,
and every tenant-owned table is rebuilt in one migration. The poll and the graph build run per
tenant and per installation, sequentially at this scale. The context graph's `team` source
becomes one per tenant. An invitee may see private repositories' issue titles, plans and code
structure that GitHub would hide from them, and may create issues and map pull requests there as
the App; the members page says so. Covered by the `web-server` O-row
(`rubato.web.rejected{reason=other_tenant|not_invited}`), the `server-github` O-row
(`rubato.github.webhook.deliveries{outcome=unattached}`, `rubato.github.rate_limit.remaining`
per installation) and the `companion-server` O-row (`rubato.companion.refused{reason=not_member}`).

## Supersedes

0022 (access to the installation decides who is on the team); in part 0044 (everyone who may log
in sees every workspace; a `default` workspace is required); in part 0048 (the poll's union is
per tenant); in part 0075 (the member's own token vets a new map source); in part 0086 (every
source goes to every companion whose owner has a fresh team record; 64 sources at most).
