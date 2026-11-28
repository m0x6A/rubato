Feature: Findings
  What the map and the plans disagree about, and where plans meet, shown and never silent.

  @findings.seam-collision
  Scenario: Two plans touching the same seam show on that seam
    Given two issues in different repositories whose plans both touch one seam
    When both plans come back
    Then a seam collision on that seam names both issues

  @findings.unknown-part
  Scenario: A plan naming a part the map does not have is a finding
    Given a plan that names an id the map does not have
    When the server reads it
    Then a finding names the plan and the unknown id

  @findings.own-view
  Scenario: Findings have a view of their own
    Given the map and the plans disagree about something
    When a person looks at any view
    Then the panel says how many findings there are, quietly, and does not list them
    When they follow that link
    Then they see every finding, and Escape returns to the list
