Feature: New issue from the map
  A person starts an issue from the part or seam it is about (ADR 0025).

  @new-issue.pinned-where-begun
  Scenario: An issue created from a part is pinned on that part
    Given a team member who selected a part
    When they fill in "New issue here…" and create it
    Then the new issue opens, Planned, pinned to that part

  @new-issue.created-as-the-app
  Scenario: The issue is created in GitHub as the App installation
    Given a team member logged in through the App, and an installation that may create issues
    When they create an issue from a part
    Then GitHub gets the issue from the App installation
    And the new issue opens, pinned to that part
