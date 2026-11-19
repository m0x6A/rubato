# 0083. Observed traffic says when a quiet part or seam was last seen, and which are not instrumented; the lens draws three states

- **Status:** accepted
- **Date:** 2026-11-19
- **Decided by:** Mattias Jonsson (m0x6A), approved in the Claude Code session 2026-11-19 ("build it", then "merge all and fix all I need to do"), with plan #300 (PR #301); drafted by agent (Claude Code, claude-opus-5-5); the three states are Mattias Jonsson's choice, 2026-11-19
- **Seam:** web-server
- **Issue:** m0x6A/rubato#300

## Context

ADR 0039 carries calls in one window. A part with no calls in it reads as silent, whether it went quiet, runs a few times a day (the skill, plan-check), or cannot be observed at all (a part without `service`, ADR 0059). Since #298 the lens fades all three alike. Plan: `docs/plans/300.md`, design §4.

## Decision

`TrafficReport` gains two additive fields:

- `seen`: every edge a lookback saw, each with the shortest lookback, `within_secs`;
- `lookback_secs`: the longest lookback asked, absent until one has answered.

A lookback task beside the Prometheus adapter's fetch (ADR 0040) fills them. It has its own timeout, and runs on the first fetch after the source starts and then every tenth. It asks instant queries over 6 h, 1 d and 7 d, skipping any no longer than the window. A failing lookback keeps the previous result, or none, and never makes the report failing: the window's report alone decides fresh, stale and failing.

`derive_traffic` adds, on a fresh report only:

- `ObservedTraffic.last_seen`: for each part and declared seam with no calls in the window, the smallest `within_secs` it was seen in;
- `ObservedTraffic.lookback_secs`: as the report's;
- `ObservedTraffic.not_instrumented`: parts without `service`, and seams with an end without one.

Absent lookback data says nothing. A quiet part is "not seen" only when a lookback answered and did not see it.

The lens draws three states:

- **observed**, as ADR 0039 and #298;
- **quiet**, with "last seen" in the bucket's words. Plain when no lookback has answered; faded only when one has, and did not see it.
- **not instrumented**, neutral and hatched, never faded.

What stands from ADR 0039: unknown is never quiet, and traffic is never an input to progress, pins, plans or approvals. From ADR 0059: `SeamQuiet` still needs both parts to carry `service`.

## Alternatives considered

- **A range query with steps for an exact last time**: costs more on the backend and gives a time that is still only as fine as the step.
- **Two states only (observed, not instrumented)**: a skill run an hour ago would look like a part that never runs.
- **Keeping the last time seen on the server across restarts**: state in SQLite for a value the backend already holds.

## Consequences

- Three more queries every tenth fetch, the 7 d one the heaviest.
- The overview carries a little more per part and seam.
- An old server's overview has neither field and draws as #298 does.
- O-row: log line `traffic lookback` with `rubato.traffic.seen` and `rubato.outcome`.

## Supersedes

None. Extends 0039 and 0040.
