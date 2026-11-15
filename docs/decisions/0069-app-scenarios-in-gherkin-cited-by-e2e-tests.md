# 0069. Write the app's user scenarios in Gherkin as the spec, and have every e2e test cite the scenarios it covers

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, claude-opus-5-5, issue #198) — an agent may propose; a human accepts
- **Seam:** none (a data format and a way of working: the scenario files, the scenario ids and the results files the coverage report reads)
- **Issue:** m0x6A/rubato#198

## Context

The end-to-end suites (`e2e/run.ts` for the server and companion, `web/e2e/*.spec.ts` for the web
app) test a lot, but nothing says which user scenarios Rubato is meant to support, and nothing
shows which of them the tests cover. A green run says the tests pass, not that the important flows
are tested. The companion already specifies itself in Gherkin, run by cucumber-rs
(`crates/rubato-companion/features/*.feature`); the app as a whole has no such list. Issue #198
asks for the list, an explicit link from each test to it, and a coverage report in CI and locally.

The map already has *scenarios* (ADR 0024): journeys over the map's flows, under `scenarios:` in
`system-map.yml`, played on the map. Those say how data moves between parts; the user scenarios
here say what a person can do with the app and see. They are different things with different ids.

## Decision

The app's user scenarios are written in Gherkin in a top-level `features/` folder, one
`<capability>.feature` per capability. Each scenario carries exactly one id tag
`@<capability>.<slug>` (lowercase, the capability the file's name), which never changes once
merged; a renamed scenario keeps its id, and a removed one's id is not reused. These ids always
hold a dot, so they cannot be mistaken for map ids or map scenario ids (ADR 0024). The files are
the spec only: no step definitions run them.

Each e2e test cites the scenarios it covers by those ids: a Playwright test through its `tag`
detail, an `e2e/run.ts` check in its entry of `e2e/run-checks.ts`. A test that asserts no
behaviour (the screenshot writer) is tagged `@tooling` instead. A `must fail:` test, which proves
that a check catches what it should, cites the scenario of that check and also carries
`@selftest`: its ids must exist, but it is listed apart and does not count as covering.

`pnpm scenarios check` fails on an id a test cites that no scenario has, on a test that cites no
scenario and is not `@tooling`, and on a malformed or duplicated scenario id; it only warns on a
scenario no test cites. It prints how many scenarios and tests it checked.

`pnpm scenarios report` reads the suites' results (`e2e/results/playwright.json`, Playwright's
JSON reporter; `e2e/results/run.json`, written by `e2e/run.ts` as
`{ suite, tests: [{ key, title, file, scenarios, status, checked, problems }] }`), falls back to
the list of tests with status *not run* for a suite with no results, and writes a Markdown table,
to `e2e/results/scenarios.md` and to the GitHub job summary. Each scenario is marked by the first
rule that holds:

1. ⬜ *not covered*: no test cites it;
2. ✅ *covered and passing*: every citing test passed;
3. ⏸ *not run*: no citing test ran (its suite has no results here);
4. 🟡 *partly covered*: anything else, with the counts (`p of m passed; f failed, s skipped, r not run`).

*Partly* is about the run, not about how much of the scenario the tests reach, which the issue's
wording also allowed: the report cannot know that, and review of the tags is where it is judged.
`report` exits non-zero only on what fails `check`; a failing test fails its own job.

New features come with a scenario, and new e2e tests cite theirs.

## Alternatives considered

- **Gherkin driving the tests (cucumber step definitions for Playwright and `e2e/run.ts`)**: a
  second test framework and a step layer over specs that already read well; heavier, and the
  wording of every scenario would bend to reusable steps.
- **Scenarios under `docs/scenarios/`**: they are a spec the tests are checked against, not
  prose; `features/` matches the companion's folder name.
- **One list: write the user scenarios as map scenarios (ADR 0024)**: a map scenario is a path of
  flows across seams; most user scenarios (log in, switch workspace, reduced motion) cross none,
  and the map would fill with what is not structure.
- **An uncovered scenario fails the check**: most of the app's scenarios are written ahead of
  their tests today; a failing check would push toward deleting scenarios or writing thin tests.
  Warn first; a later ADR can make it fail once coverage is high.
- **An HTML report**: more to build; Markdown shows in the job summary for free. The map view
  could show coverage later.
- **Scenario ids as tags in the test title**: they would clutter every report and trace name; the
  `tag` detail is Playwright's own field for this.

## Consequences

The list of what Rubato supports is written down and versioned, and a review can ask "which
scenario?" of any new feature. A test that tests nothing named, or names a scenario that is gone,
fails the node job. The report shows the gap honestly, so many scenarios start uncovered. Watch
that ids stay stable: the check cannot tell a renamed id from a new one, so a rename shows as one
scenario uncovered and the tests that cited it failing the check, until they are updated. A
feature scenario may later name the map scenario it walks; not now. The companion's own Gherkin
suites are unchanged and are not part of the report. No seam changes, so no O-row.

## Supersedes

none
