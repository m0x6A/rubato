Feature: The system map
  One shared map above the repositories: parts, the seams between them, and the flows each seam
  carries, read from the versioned map file.

  @map.every-part
  Scenario: The map shows every part of the map file
    Given a map file with parts and seams
    When a person opens Rubato
    Then every part of the map is on the canvas
    And the map's name and version are shown

  @map.by-repo
  Scenario: By repo groups the parts by the repository behind them
    When a person turns on By repo
    Then each repository's parts are drawn inside one box
    And external parts stay outside every box

  @map.shared-layout
  Scenario: A part moved by one person is in the same place for everyone
    Given two people with the map open
    When one of them drags a part to a new place
    Then the other sees the part in its new place
    And it is still there after a reload

  @map.flow-lens
  Scenario: The data flow lens shows the flows each seam carries
    When a person picks the data flow or control flow lens
    Then each declared flow of that kind is an arrow with a head, in its direction
    And a seam without such a flow is a faint plain arrow

  @map.inspect-part-and-seam
  Scenario: Selecting a part or a seam explains it
    When a person selects a part
    Then the inspector says what the part is and which flows reach it
    And what is not next to it steps back
    When they follow one of its seams
    Then the inspector says what the seam carries and who is woken when it breaks

  @map.play-journey
  Scenario: A journey plays step by step over the map
    Given the map declares the journey "start a session on an issue"
    When a person plays it
    Then each step lights the arrow and the two parts it crosses
    And they can step forward and back
    And nothing covers the step on the canvas

  @map.ghosts-and-discrepancies
  Scenario: Observed traffic the map does not declare shows as ghosts and discrepancies
    Given a traffic report with calls to services the map does not have
    When a person picks the Traffic lens
    Then the unknown services show as ghosts in their own band
    And the declared parts stay where they were
    And the map-versus-territory list counts each discrepancy

  @map.dismiss-discrepancy
  Scenario: A person dismisses a discrepancy for the workspace
    Given a discrepancy between the map and the observed traffic
    When a person dismisses it
    Then it leaves the list for everyone in the workspace
    And the map file is not changed
