# 0069. Write the app's scenarios in Gherkin as the spec, and have every e2e test cite the scenarios it covers

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

## Decision

The app's user scenarios are written in Gherkin in a top-level `features/` folder, one
`<capability>.feature` per capability. Each scenario carries exactly one id tag
`@<capability>.<slug>` (lowercase, the capability the file's name), which never changes once
merged; a renamed scenario keeps its id, and a removed one's id is not reused. The files are the
spec only: no step definitions run them. Each e2e test cites the scenarios it covers by those ids:
a Playwright test through its `tag` detail, an `e2e/run.ts` check in its entry of
`e2e/run-checks.ts`. A test that asserts no behaviour (the screenshot writer) is tagged `@tooling`
instead. `pnpm scenarios check` fails on an id a test cites that no scenario has, on a test that
cites no scenario and is not `@tooling`, and on a malformed or duplicated scenario id; it only
warns on a scenario no test cites. `pnpm scenarios report` reads the suites' results
(`e2e/results/playwright.json`, Playwright's JSON reporter; `e2e/results/run.json`, written by
`e2e/run.ts` as `{ suite, tests: [{ title, file, line, scenarios, status, problems }] }`) and writes
a Markdown table, to `e2e/results/scenarios.md` and to the GitHub job summary, that marks each
scenario covered and passing, partly covered (some citing test failed, was skipped or did not
run), or not covered. New features come with a scenario, and new e2e tests cite theirs.

## Alternatives considered

- **Gherkin driving the tests (cucumber step definitions for Playwright and `e2e/run.ts`)**: a
  second test framework and a step layer over specs that already read well; heavier, and the
  wording of every scenario would bend to reusable steps.
- **Scenarios under `docs/scenarios/`**: they are a spec the tests are checked against, not
  prose; `features/` matches the companion's folder name.
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
fails the node job. The report shows the gap honestly, so most scenarios start uncovered. Watch
that ids stay stable: the check cannot tell a renamed id from a new one, so a rename shows as one
scenario uncovered and the tests that cited it failing the check, until they are updated. The
companion's own Gherkin suites are unchanged and are not part of the report. No seam changes, so
no O-row.

## Supersedes

none
