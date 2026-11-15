# A fixture for e2e/scenarios.test.ts: one scenario per coverage state.
Feature: Logging in
  A description line, not a step.

  @auth.all-pass
  Scenario: Every citing test passed
    Given a test that passed
    Then the scenario is covered

  @auth.one-failed
  Scenario: One citing test failed
    Given a test that passed and one that failed
    Then the scenario is partly covered

  @auth.nobody
  Scenario: No test cites it
    Given no test
    Then the scenario is not covered

  @auth.selftest-only
  Scenario: Only a selftest cites it
    Given a must-fail test
    Then the scenario is not covered

  @auth.never-ran
  Scenario Outline: Its tests did not run
    Given a test with no result <n>
    Then the scenario is not run

    Examples:
      | n |
      | 1 |
