# 0074. Learn about GitHub changes from the App's webhooks; the poll becomes a slow catch-up

- **Status:** accepted
- **Date:** 2026-11-15
- **Decided by:** agent (delegated by Mattias, 2026-11-15), accepted with plan #208 at d63e57f; drafted by agent (Claude Code, claude-opus-5-5, issue #208)
- **Seam:** server-github
- **Issue:** m0x6A/rubato#208

## Context

On 2026-11-15 the deployed server had used up the GitHub App installation's hourly budget, and
issue and pull request state stopped updating until the reset. The poll asks GitHub about every
tracked issue every 60 s (ADR 0023, 0048), and most of those reads find nothing new. GitHub can
tell the server what changed instead: the App can send webhooks. Design:
`docs/plans/208-github-webhooks/design.md`.

## Decision

`rubato-server` takes GitHub App webhook deliveries at `POST /webhooks/github` when
`RUBATO_GITHUB_WEBHOOK_SECRET` is set, and only then; without it the path answers 404 and the
poll is exactly as ADR 0023 and 0048 say. A delivery counts only when its `X-Hub-Signature-256` is
the HMAC-SHA256 of the raw body under that secret, compared in constant time. The signature is
its only authentication: the path is outside the same-origin write check of ADR 0066, and it is
the one path a deployment needs to expose to GitHub (through a tunnel, a reverse proxy or
otherwise; Rubato depends on no relay product). A verified delivery is recorded once by its
`X-GitHub-Delivery` id and once by the SHA-256 of its body, with no payload: the event, action, repo, the issue number it resolves to
and the branch it names. A redelivery, or a captured body under fresh headers, answers 200 and changes nothing. The
server answers before it reads GitHub; the poll loop, which makes every other GitHub read and is
the only writer of what it reads, then refreshes only what recorded deliveries name, through the same per-issue reads: the issue (`issues`,
`issue_comment`), the issue whose `issue/` branch a pull request, check suite, completed check run
or push names, the repo's open issues listing for `issues`, its open pull requests for
`pull_request` and checks on any branch, and the map for a push to a workspace's map branch.
Tracked issues not read since start still get their first read on the next 60 s tick. With webhooks on,
the full poll of ADRs 0019, 0023, 0032, 0034 and 0048 runs every `RUBATO_GITHUB_CATCHUP_SECS`
(default 900, 300 to 3600) as a catch-up for missed deliveries, always on the full lane; the
open issues listing of every repo and the database-only full-lane work keep the 300 s lane of
ADR 0023, and local work on the loop (rooms, gauges, the rate-limit read) keeps its 60 s tick.
The cadence is kept by clock deadlines, so a delivery that wakes the loop moves none of them. A change the catch-up finds
for an issue it has already read is counted, so a hook that stopped delivering is seen. Per-issue
comment reads (`Review:`, `Approved:`, `Decision:`) stay as they are until the cutover
(ADR 0054); this ADR removes no read the cutover still needs.

## Alternatives considered

- **Keep polling, only slower**: every state is late for everyone, and the budget still grows
  with tracked issues.
- **A relay the server pulls from (smee-style)**: a third party in the path that sees every
  payload, and a dependency to run; the signature check works the same behind any ingress.
- **Make the whole server public**: widens what is exposed from one signed path to every route.
- **Store payloads**: larger rows and issue bodies kept outside GitHub, for nothing the refresh
  needs; the refresh reads current state from GitHub anyway.
- **Refresh in the request handler**: a second writer racing the poll loop on the same rows, and
  GitHub's 10 s delivery timeout tied to GitHub's own latency.

## Consequences

An issue's pull request, checks, plan and `Map:` line show within seconds of the change instead of
up to 60 s (300 s for comments), and the catch-up at about 30 tracked issues costs a few hundred
calls an hour instead of most of 5,000. A missed delivery is late by at most one catch-up interval.
Turning webhooks on is configuration only: the secret, the App's webhook URL and its events. The
`server-github` O-row covers it: `rubato.github.webhook.deliveries` by event and outcome,
`rubato.github.webhook.last_accepted_at`, `rubato.github.webhook.pending_oldest_seconds`,
`rubato.github.catchup.last_change_at` and the alerts `GitHubWebhooksSilent` and
`GitHubWebhooksStuck`.

## Supersedes

While webhooks are on, and only then (with webhooks off each stands whole):

- ADR 0019 "On every poll tick the server lists the open issues": every 300 s full lane, and for
  the repo an `issues` delivery or an untracked named issue names.
- ADR 0023 "On every tick … lists … branches … and the open pull requests", "`checks` is read every
  tick", and the 300 s full lane for per-issue reads: on each catch-up, and for what a delivery
  names; the plan-head rule, the settled closed issue and the gone-branch rule stand.
- ADR 0032 "On every tick … lists … refs": on each catch-up, and for an untracked issue a delivery
  names.
- ADR 0034 "On every tick … lists … open pull requests": on each catch-up, and for the repo a pull
  request or check delivery names.
- ADR 0048 "Each tick the server reads the union": each catch-up reads the union; a delivery's
  refresh reads its repo and forgets nothing of the others.
- ADR 0066 "Every request that is not GET … passes one check": except `POST /webhooks/github`,
  whose authentication is the signature.
