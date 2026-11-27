# 0088. Operators create tenants and attach installations; tenant admins invite members

- **Status:** proposed
- **Date:** 2026-11-27
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #345), drafting; operator-created tenants on a web admin page, tenant admins who manage members, and treating a reused login as a new person are Mattias Jonsson's choices, 2026-11-27
- **Seam:** web-server, server-github
- **Issue:** m0x6A/rubato#345

## Context

ADR 0087 makes membership an invite. Someone has to create tenants, decide which installation is
whose, and send invites. ADR 0075 chose no admin role, "a role model this lab does not have"; a
server shared by teams that do not know each other needs one. Design:
`docs/plans/345-tenants/design.md` §4.4 and §5.3.

## Decision

An **operator** is a GitHub user id listed in `RUBATO_OPERATORS`; the role cannot be granted from
inside the app. Operators use `/api/admin/…` and the admin page: list the App's installations as
the App, create a tenant, attach and detach installations (detach refused while a workspace names
one of its repositories), name a tenant's first admin, delete a tenant with no workspaces. An
operator sees tenants' names, installations and member counts, not their data. A tenant **admin**
invites a GitHub login, which the server resolves to its user id at invite time, changes roles
and removes members; the last admin cannot be removed or demoted. Only admins create, edit and
delete the tenant's workspaces; members use them. A login is refused a session unless it is an
operator or a member of at least one tenant (`not_invited`). Removing a person's last membership
ends their web sessions, deletes their GitHub tokens and revokes their pairings in one
transaction. Removing a person from any one tenant also dismisses their agents
and pending requests in its rooms and drops their queued issues there. Membership is read on every
tenant request and on every companion message that acts in a tenant, so a change takes effect at
once. A login whose GitHub user id differs from the one stored is a new person: before anything
else the old one's sessions, tokens and pairings are revoked, and no membership carries over.
Detach, purge and tenant delete follow the lifecycle of the design (§3.3): delete is refused
while a tenant has workspaces or installations, and a tenant id is not reused while rows of it
remain.

## Alternatives considered

- **Operator on a host command line**: no operator surface on the internet, but the operator
  could not work from a browser; the admin routes answer 404 to everyone else instead.
- **Operator only, no tenant admins**: every invite would wait for the operator.
- **Invites by login string**: a renamed or reused login would carry someone else's access.
- **Any member may edit workspaces** (ADR 0075 as written): fine for one team, too wide for a
  visitor in a demo tenant.

## Consequences

New slices `tenants` (members) and `admin` (operator), and web features for both. Invites never
expire; an invite not yet used shows as *invited*. A self-hosted single team sets itself as
operator and admin and works as before. Covered by the `web-server` O-row
(`rubato.tenant.member_changed`, `rubato.web.rejected{reason=not_invited}`).

## Supersedes

In part 0075 (no admin role; any member may create, edit or delete a workspace).
