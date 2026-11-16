Feature: Broken on purpose

  Scenario: No id tag
    Given nothing

  @auth.one @auth.two
  Scenario: Two id tags
    Given nothing

  @map.wrong-file
  Scenario: Id of another capability
    Given nothing

  @auth.Not_Lowercase
  Scenario: Malformed id
    Given nothing

  @auth.twice
  Scenario: First of a duplicate
    Given nothing

  @auth.twice
  Scenario: Second of a duplicate
    Given nothing

  @auth.no-steps
  Scenario: No steps
