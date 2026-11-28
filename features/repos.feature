Feature: Connected repositories
  The repositories behind the map's parts, and what each provides.

  @repos.lists-map-repo
  Scenario: The Repos tab lists the map's repository as connected
    When a person opens Repos
    Then the map's repository is listed as connected

  @repos.what-each-provides
  Scenario: A repository says which parts it provides
    When a person opens Repos and opens a repository
    Then the repository says how many parts it provides
    And lists them

  @repos.context-graph-age
  Scenario: The Repos tab says how fresh each repository's context graph is
    Given the server builds the context graph
    When a person opens Repos
    Then the map's repository says the head its graph was built at, when it was last found current and its size
    And the workspace's map and the team's facts say the same of theirs
