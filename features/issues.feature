Feature: Issues
  GitHub is the source: Rubato lists the issues that are open there, and keeps the finished ones
  where a person can ask for them.

  @issues.open-only
  Scenario: An issue closed on GitHub leaves the default lists
    Given a tracked issue with a plan
    When the issue is closed on GitHub without a merged pull request
    Then the issue is not in the Issues list, and the Issues count does not include it
    And its plan is not among the proposed or open plans
    And the map shows it on no part

  @issues.done-view
  Scenario: Finished issues are listed under Done
    Given an issue closed on GitHub and an issue whose pull request merged
    When a person switches the Issues list to Done
    Then both are listed, the closed one marked closed
    And a finished issue's detail offers nothing that starts work
