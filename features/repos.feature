Feature: Connected repositories
  The repositories behind the map's parts, and what each provides.

  @repos.lists-map-repo
  Scenario: The Repos tab lists the map's repository as connected
    When a person opens Repos
    Then the map's repository is listed as connected

  @repos.what-each-provides
  Scenario: The Repos tab says which parts each repository provides
    When a person opens Repos
    Then each repository says how many parts it provides
    And opening it lists them
