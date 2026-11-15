# 0065. `Decision:` comments count only from team members, like `Review:` and `Approved:`

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** m0x6A, accepted 2026-11-15 by approval comment https://github.com/m0x6A/rubato/issues/164#issuecomment-6892511479 (self); drafted by agent (Claude Code, claude-opus-5-5, issue #164)
- **Seam:** server-github
- **Issue:** m0x6A/rubato#164

## Context

ADR 0014 records decisions as issue comments whose first line is `Decision:`, and the server
records every comment of that shape. ADR 0027 later made `Review:` and `Approved:` count only from
team members, because the repo is public. `Decision:` was left out: the poll checks the author only
for reviews and approvals (`crates/rubato-server/src/github_poll/poll.rs:321-323`), so anyone on
GitHub can add a decision that Rubato shows next to the team's own, and teammates and agents may act
on it. The skill's `plan.ts wait` already reports a decision only from a trusted author
(`skills/rubato/scripts/lib/comments.ts:97`), so the two readers disagree today.

## Decision

The server records a `Decision:` comment only when its author passes `trusted_author`: not a Bot,
and association OWNER, MEMBER or COLLABORATOR, the rule of ADR 0027. A `Decision:` comment from
anyone else is ignored, logged as `decision ignored` with the comment id, the author and the reason
(`bot` or the association), and any row already recorded for that comment is removed. The shape of
a decision (ADR 0014) is unchanged.

## Alternatives considered

- **Record it and mark it as from outside the team**: the record would still look official on the
  map, and every reader would have to check the mark; nothing in v0 needs outside decisions.
- **Keep it as is**: the server and the skill would keep reading the same comment differently.

## Consequences

A decision written by someone outside the team (a contributor without collaborator access) is no
longer shown; a teammate restates it if it stands. Decisions are read on the same full poll lane
as before, with no extra GitHub call. O-row: server-github (`decision ignored` log line, described
on the Approvals panel).

## Supersedes

none. It narrows which comments ADR 0014's decision records; the rest of 0014 stands.
