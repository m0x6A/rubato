---
feature: 49-poll-cost
issue: 49
title: "Server: the per-issue poll costs about six GitHub calls per tick"
track: full
status: implemented
date: 2026-11-12
author: Mattias Jonsson
produced_by: { model: claude-opus-5-5, model_name: Claude Opus 5.5, tool: Claude Code, effort: unknown }
review: { stage1: "4 blockers fixed", by: design-reviewer, date: 2026-11-12 }
approval: "given up front by Mattias Jonsson on 2026-11-12 via the goal 'finish issues 40-49'; he reviews at the PR"
touches: [rubato-server]
adrs: [0023-poll-github-per-repo-and-by-lane]
specs: []
---

# Server: the per-issue poll costs about six GitHub calls per tick

Issue: #49 · Track: full (it changes how the server reads GitHub, seam `server-github`, and the
poll loop's cadence; both are full-track triggers in `.claude/profiles/feature-workflow.md`) ·
Branch: `issue/49-poll-cost` · Plan: [../49.md](../49.md)

## 1. Problem

`tasks::poll_issue` (`crates/rubato-server/src/tasks.rs`) makes, for every tracked issue on every
tick: `issue`, `branches_with_prefix` (which lists **every** branch of the repo, all pages),
the plan `file`, `pull_for_branch`, `checks`, `issue_comments`. At the default 60 s tick that is
about 360 calls per hour per issue, so about a dozen tracked issues use a 5000-per-hour budget.
Closed pinned issues stay tracked forever, so the cost only grows.

**Acceptance criteria** (from the issue)

- [x] **AC1** — the call-order test asserts a smaller list per issue.
- [x] **AC2** — with 30 tracked issues at the default tick, `rubato.github.rate_limit.remaining`
  stays above 10%.

Must not break (from the issue): the derived progress states and their latency for an issue
someone is working on; `GitHubRateLimitLow` still fires when the budget is low.

## 2. Spec

### 2.1 What the change does

- **S1** — Per repo, per tick, at most one branch listing and one open-pull-request listing,
  shared by every tracked issue of that repo, and each only when an issue of the repo needs it.
  The per-issue `branches_with_prefix` and, for an issue whose pull request is open,
  `pull_for_branch` calls go away.
- **S2** — An issue that appears in this tick's open-issues listing (ADR 0019) takes its title,
  URL and state from the listing; no `issue` call.
- **S3** — The plan file is fetched only when the issue branch's head commit or the plan path
  differs from the last successful fetch since the server started.
- **S4** — Two lanes. A **full** tick (the first tick after start, then the first tick once
  300 s of intervals have passed since the last full one, whatever the interval) reads
  everything; a **fast** tick skips `issue_comments` and the `pull_for_branch` look-up for a
  branch that has never had a pull request. An issue seen for the first time since the server
  started always gets a full pass.
- **S5** — An issue whose pull request is merged or closed (stored) is not looked up again unless
  a pull request on its branch shows up in the open listing (reopened, or a new one). A stored
  open pull request that leaves the open listing is looked up on that same tick, by the stored
  pull request's own branch when the branch is gone (deleted at merge), so *Done* is not delayed.
- **S6** — A closed issue is settled when it is closed, has no stored open pull request and no
  live session: once its close is seen it gets one last full pass (retried until it succeeds),
  then no calls of its own until it appears in an open listing again. A closed issue with an open
  pull request or a live session keeps its branch, plan and pull request reads. For a repo the
  map does not list (or whose listing failed or was cut), the issue is read with `issue` on full
  ticks only.
- **S7** — Each tick logs one line `poll tick` with `lane`, `tracked`, `settled`, `failed` and
  `repos` (`info` on the full lane, `debug` on the fast one); an issue that settles or reopens
  logs one `info` line with `rubato.issue` (`issue closed; settled until it reopens`,
  `issue reopened; polled again`).

### 2.2 How we'll know it works

| Proves | Evidence | Kind |
|---|---|---|
| S1, S2, S3, S4, AC1 | `poll_makes_the_same_github_calls_per_issue` (integration) asserts the full-tick list (11 calls for the three issues, was 16) and the fast-tick list after it (5) | integration |
| AC2 | `thirty_tracked_issues_stay_inside_the_rate_limit_budget`: one repo, 30 pinned issues each with a branch, a plan and an open green pull request (31 branches, 30 open issues, 30 open pull requests: every listing is one page), one simulated hour of ticks at the default 60 s with the `LaneClock` that `run` uses; the fake's budget is 5000 minus calls made. Today: 2430 calls, 2569 left. Made to fail on purpose with the plan cache off and every tick full: 5640 calls, 0 left | integration |
| S4 | `tasks::tests::the_full_lane_comes_by_elapsed_time` at 60, 45 and 300 s | unit |
| S5 | `a_pull_request_that_leaves_the_open_listing_is_read_at_once`; `a_merge_that_deletes_the_branch_is_done_on_the_next_tick` | integration |
| S6 | `a_closed_issue_is_settled_until_it_reopens`; `a_closed_issue_with_an_open_pull_request_still_reaches_done`; `a_failed_last_pass_is_retried` | integration |
| S3 | `the_plan_file_is_read_again_only_when_the_branch_moves` | integration |
| S7 | read in review; no test captures the lines (the integration harness has no log capture) | — |

### 2.3 What it must not break

| # | Must keep working | Guarded by |
|---|---|---|
| N1 | *In review* (open, not draft, green) as soon as checks go green: `checks` is still read every tick for every open pull request | AC1 lists (fast tick has `checks`) |
| N2 | *Done* on the tick after the merge, also when the branch is deleted at merge or the issue was closed first | S5 and S6 tests |
| N3 | *Designed* on the tick after the plan is pushed (the push moves the branch head) | S3 test |
| N4 | `GitHubRateLimitLow`: `rate_limit` is read at the end of every tick, both lanes | AC1 lists (both end with `rate_limit`); the rule itself is unchanged |
| N8 | Known and accepted: a pull request opened **and** merged between two fast ticks on a branch that never had one is seen on the next full tick (at most 300 s), unless the issue closes, which forces a last pass | — (A5) |
| N5 | A failed branch listing skips the issue's tick with one warning, as before; a failed or cut open-pull-request listing falls back to `pull_for_branch` per issue | `poll_falls_back_when_the_pull_listing_fails` |
| N6 | The map-draft proposal's own `pull_for_branch` is untouched | existing map-draft integration tests |
| N7 | Slice rules: `github_poll` names no slice; only `tasks.rs` names `plans` | `slices::*` layout test |

### 2.4 How we'll debug it at 3 am

| # | When this fails | How we find out | What localises it | What it tells us · first action |
|---|---|---|---|---|
| O1 | An issue stays *In review* or *Building* after its pull request merged | the issue card on the overview | the issue's `issue closed; settled …` / `issue reopened …` lines; `rubato.github.call` spans with `rubato.github.endpoint = pulls` | a settle line with no later `pulls` span: it settled while work was in flight · check its stored pull request and sessions; a restart forces one full pass |
| O2 | Budget burns faster than this design says | `rubato.github.rate_limit.remaining` on "GitHub calls · Rate limit"; `GitHubRateLimitLow` | span counts by `rubato.github.endpoint` in Tempo; `failed` on `poll tick` | a repo with many branches (the branch listing pages), a fallback after `pull listing failed` / `pull listing cut` warnings, or other spenders of a personal token's budget (it is per GitHub user) |
| O3 | A reopened issue in a repo the map does not list stays settled for up to 300 s | issue still shown closed | `settled` on the `poll tick` line | expected (S6); add the repo to the map's `repos[]` |

## 3. Where it fits

One node, `rubato-server`, and its existing seam `server-github`. Nothing on the wire to the
companion or the browser changes; no table or migration. The GitHub port (`kernel/github`) gains
`branches(repo, prefix)` returning names with head commits (replacing `branches_with_prefix`) and
`open_pulls(repo)`; the octocrab, local-git and fake adapters implement both.

## 4. Main flow (one tick)

```
run ── lane(map_age) ──► Poller::tick(app, lane)
  list_open_issues (per map repo, ADR 0019) ─► Listed { fresh repos, open issues }
  for each tracked issue:
    meta:   in listing → upsert from listing | absent from a fresh listing → settled or one `issue` | not listed → `issue` on full lane
    settled & not first & not just closed → skip
    branch: RepoView(repo).branches()   (lazy, once per repo per tick)
    plan:   (path, head sha) changed → plans::fetch_and_store
    pr:     RepoView(repo).open_pulls() has branch → upsert + checks
            stored PR final → nothing
            stored PR open, listing failed, or full lane → pull_for_branch (+ checks if open)
    decisions: full lane, first sight or just closed → issue_comments
  rate limit, gauges, notify
```

## 5. Control flow

Any error inside one issue skips the rest of that issue, logged once (unchanged). A repo-level
listing error is kept in the tick's `RepoView`, so every issue of that repo sees the same error
without a second call.

## 6. Data flow

In: GitHub listings and per-issue reads. Out: the same rows as before in `issue_meta`,
`pull_requests`, `decisions` and `plans`. The poller keeps in memory, for the life of the process:
the issues it has seen, and the (plan path, head sha) last fetched per issue. Losing it on restart
costs one full pass.

## 7. Data model and states

Per tracked issue, derived each tick, never stored: **active** (open, or state unknown) or
**settled** (stored `github_state = closed` and absent from a fresh listing). Per stored pull
request: **open** or **final** (merged or closed). No new columns.

## 8. Affected parts

| Node | Status | What changes | Seams |
| --- | --- | --- | --- |
| rubato-server | affected | poll reads branches and open PRs per repo, meta from the open listing, plans on head change, comments and closed issues on the full lane | server-github |

## 9. Caveats to consider

- **Concurrency**: one poll loop; the poller state is owned by it. `poll_once` (tests and the
  map-draft path) builds a fresh poller and runs a full tick, as before.
- **Idempotency**: every write is still an upsert of the same rows.
- **Fork pull requests**: the open listing is matched on `head.label == owner:branch`, ignoring
  case (the map may spell the owner differently from the login), the same rule
  `pull_for_branch` uses (`head=owner:branch`).
- **Cost multiplier**: a listing call is one span but may be several pages (branches: all pages;
  open issues: up to five). AC2's fixture is one page each; a repo with hundreds of `issue/`
  branches costs more per tick (O2).
- **Cut listings**: more than 100 open pull requests in a repo, or a cut open-issues listing,
  makes that listing not fresh, and issues fall back to per-issue reads (N5).
- **Telemetry names**: no new `rubato.*` names; the `poll tick` fields are plain like
  `record_rate_limit`'s.
- **Derived, not stored**: active/settled is derived each tick.

### 9.1 Concurrency and idempotency

- **Single writer**: only the poll loop writes `issue_meta`, `pull_requests`, `decisions` (and,
  through `plans`, `plans`); `poll_once` callers (tests, nothing else in production) run a pass
  of their own with the same upserts.
- **Partial failure on the closing tick**: the `issue` read stores `closed` before the rest of
  the pass. If a later call fails, the poller keeps the issue in `last_pass` (memory) and the
  next tick treats it as just closed again; it leaves `last_pass` only when a whole pass
  succeeds. Test: `a_failed_last_pass_is_retried`.
- **Restart**: memory is lost, so every issue gets one full pass on its first tick.

## 10. Decisions

### 1. Per-repo listings and two lanes → ADR 0023 (accepted 2026-11-12)

Branches and open pull requests listed once per repo per tick; comments, closed issues and
"is there a PR yet" on a 300 s full lane. Rejected: conditional requests (ETag/304). octocrab's
typed builders do not expose response headers, so it means hand-written requests and an ETag
cache for every endpoint; worth it later for `checks` if the budget is still tight. Rejected:
polling comments on their own timer (a second cadence for one call kind).

### 2. The lane follows elapsed time, 300 s like the map reload

`LaneClock` counts intervals since the last full tick, so an interval that does not divide 300
(45 s) still gets a full tick at least every 300 s plus one interval. The map reload keeps its
own `map_age` rule, which slips at such intervals (existing behaviour, left alone here).

### 3. Poller state in memory, not in the database

A restart costs one full pass (about the old per-tick cost once), and no migration.

## 11. Out of scope / Open questions

- ETag caching (D1).
- `matching-refs` instead of listing every branch: one call per repo whatever its branch count;
  left out until checked against a repo with many branches.
- Removing closed issues from tracking altogether (a pin decision, not a poll one).
- A listing of all pull requests by update time (`state=all&sort=updated`) would close N8; left
  out to keep one listing kind per repo.
- An issue that answers 404 (deleted, transferred, or the local git stand-in, which has no
  issues) is read with `issue` on every tick of a listed repo; rare in production.
- Reading the plan at the head sha instead of the branch name, so the cache key and the content
  always match (a push between the listing and the read refetches on the next tick anyway).

## 12. Review outcome

design-reviewer, 2026-11-12, one pass over design, plan and ADR (it saw the code half written):
4 blockers, 3 should-fixes; all blockers taken.

- **Fixed (blockers):** Done after a merge that deletes the branch (S5, `poll_pr_of_gone_branch`);
  a closed issue with an open pull request or a live session is not settled (S6); the last pass
  is retried until it succeeds (§9.1, `last_pass`); records: the ADR stays *proposed* and the PR
  says it waits for its acceptance.
- **Fixed (should-fix):** the lane by elapsed time, not by `map_age` (D2, tested at 45 s); owner
  matched ignoring case; `poll tick` on every tick plus settle/reopen lines; O1's first action is
  reading those lines, not a restart; the AC2 fixture's page counts stated.
- **Not taken:** a counter of HTTP pages per tick (`requests`) and `budget_used`: they need a
  count inside the adapter; `rubato.github.call` spans by endpoint answer the same question.
  `state=all` listing for N8 (§11). Settling 404s (§11).
