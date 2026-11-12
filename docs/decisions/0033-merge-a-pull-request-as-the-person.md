# 0033. Merge a pull request from Rubato as the person, through their user-to-server token

- **Status:** proposed
- **Date:** 2026-11-12
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #82); choices Q1–Q4 answered by Mattias Jonsson 2026-11-12; a human accepts
- **Seam:** server-github, web-server
- **Issue:** m0x6A/rubato#82

## Context

Issue #82 asks that a person can merge a pull request of a connected repo from Rubato, as
themselves. ADR 0030 stores each person's GitHub App user-to-server token and says the only write
made with it in v0 is the approval comment; it names merging as a follow-up that needs its own ADR.
The App already has Contents and Pull requests read and write (ADR 0022, `docs/QUICKSTART.md`), so
no permission changes. Design: `docs/plans/82-pulls-in-rubato/design.md`.

## Decision

A second write is made with the person's token: `PUT /repos/{o}/{r}/pulls/{n}/merge`, from a new
server slice `pulls`, on a request the person made in the browser (`POST /api/pulls/merge`). The
request carries the head sha the person saw, and Rubato sends it to GitHub as `sha`, so GitHub
merges only that head and answers 409 if it moved. `merge_method` is one the repo allows
(`allow_merge_commit`, `allow_squash_merge`, `allow_rebase_merge`, read with the person's token),
chosen by the person, default squash when allowed. Retries are off and the call times out after
10 s, as for the approval comment. Before anything is sent, Rubato re-reads the pull request and
refuses when it is closed, a draft or from a fork; the head changed; checks are pending or red, or
there are none at the head while the base branch's head has some (CI has not started); `mergeable`
is false or not yet computed; the person has no push permission on the repo; the method is not
allowed; no token is stored; or the pull request's issue (from its branch name, ADR 0006) has a plan
and no counted approval names the plan **at the head being merged**. The plan file is read at that
head, its approval sha computed as in ADR 0027, and a trusted `Approved:` must name it while the
plan fits the map (ADR 0029). The issue has a plan when the file is at the head, Rubato holds a plan
row for it, or it has a `Review:` or `Approved:` comment; only a pull request whose issue has none
of these is gated by GitHub's branch protection alone. The refusal rule is one pure function in
`rubato-protocol`, used by the view and the request alike. A merge is attempted at most once per
(repo, pull request, head sha), recorded in a table `pr_merges` owned by `pulls` (`opening`,
`merged`); an existing row is answered before any check; a request whose outcome is unknown stays
`opening` until the poll sees the merge, the pull request is seen still open at that head on two
complete listings, or 600 s pass. GitHub's own refusal (405, 403, 409, 422) is reported as
`github_refused` with its message and leaves nothing behind. Rubato never deletes the head branch.
The token still never leaves the server. On acceptance, ADR 0030's status becomes "accepted; v0
write list extended by 0033".

## Alternatives considered

- **Merge as the App**: GitHub would record the App as the merger; the audit must name the person, and branch protection's "who may merge" would not apply to them.
- **Link to the pull request on GitHub**: no write, but not "merge from Rubato".
- **Always require a counted plan approval**: blocks trivial-track pull requests, which by the skill have no plan (Q2).
- **Never gate on the plan**: lets a pull request merge whose plan nobody approved, against the approval workflow of ADR 0027 and 0029 (Q2).
- **Delete the head branch after merge**: closes every stacked pull request based on it.

## Consequences

The stored token now merges code, within the person's own permissions and branch protection: a
leaked database plus key can merge as every person with a stored token (ADR 0030's exposure, now
covering merges). Merging stays a person's act (ADR 0001, 0004): nothing merges without a click,
and no agent is given the token. The plan gate makes an unapproved plan visible at the merge
button, and reading the plan at the head closes the gap where the stored plan lags the branch or
was read from another branch of a stack. The team's `main` has no branch protection today, so the
checks rule is the only gate before CI registers. A merge that changes `.github/workflows/` may be
refused by GitHub because the App has no Workflows permission; it shows as `github_refused` with
GitHub's message, and adding that permission is a separate decision. O-rows: `server-github` (span `rubato.pr.merge`, gauge `rubato.pulls.merges_opening`,
`rubato.github.calls{rubato.github.endpoint=merge}`), `web-server`
(`rubato.web.rejected{rubato.route=/api/pulls/merge}`).

## Supersedes

The list of writes as the person in 0030 only ("in v0 the only such write is the approval
comment"). The rest of 0030 stands.
