Feature: Pull requests
  Each map repository's open pull requests and issues, a pull request's view, and merging it as
  yourself (issue #82, ADR 0033, ADR 0034).

  @pulls.listed-per-repo
  Scenario: The Repos tab lists each repository's open pull requests and issues
    When a person opens Repos and unfolds a repository
    Then its open pull requests are listed with their checks
    And its open issues are listed without their bodies

  @pulls.view
  Scenario: A pull request opens with its description, checks, reviews and files
    When a person opens a pull request from the list
    Then its description shows as text, with its checks, the team's reviews and its changed files
    And a link leads to its diff on GitHub

  @pulls.merge-as-yourself
  Scenario: A person merges a pull request as themselves
    Given a pull request that is open, green, mergeable and has no unapproved plan
    When the person picks a method the repository allows and presses Merge
    Then GitHub merges it at the head they saw and records them as the merger

  @pulls.merge-refused-says-why
  Scenario: Merge is off, with the reason, when Rubato would refuse it
    Given a pull request that is a draft, red, conflicting, or whose plan is not approved at its head
    Then Merge is off and says why
