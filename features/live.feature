Feature: Live updates
  Everyone looks at the same picture: a change made by one person reaches every open browser
  without a reload.

  @live.changes-reach-every-browser
  Scenario: A change made in one browser shows in another without a reload
    Given two people with Rubato open
    When one of them changes something the team shares
    Then the other sees the change without reloading
