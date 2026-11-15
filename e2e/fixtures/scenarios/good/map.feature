Feature: The map

  Background:
    Given a map

  Rule: parts

    @map.skipped @slow
    Scenario: A citing test was skipped
      When it is skipped
      Then the scenario is partly covered
