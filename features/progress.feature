Feature: Progress
  Each issue's progress is derived from signals that already exist, never set by hand.

  @progress.planned-to-designed
  Scenario: An issue goes from Planned to Designed as its session plans
    Given an issue pinned to the map
    When a session is started on it and pushes its plan
    Then the issue shows Planned first and Designed last

  @progress.live-states
  Scenario: A working session shows the issue as Designing or Building
    Given sessions working on their issues
    Then the issues are seen Designing or Building while they work

  @progress.spend-from-transcript
  Scenario: Each session's spend comes from its transcript
    Given a session that has used tokens
    Then its issue shows the spend priced from the session's transcript

  @progress.unknown-without-heartbeat
  Scenario: A session whose companion went quiet shows as unknown, never as working
    Given a session reported as working
    When its companion stops reporting for longer than the heartbeat interval
    Then the session shows as unknown

  @progress.review-and-done
  Scenario: An issue with an open pull request is In review, and Done when it merges
    Given an issue with a plan on its branch
    When a pull request for the branch opens with green checks
    Then the issue shows In review
    When the pull request merges
    Then the issue shows Done
