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

  @map.by-repo-readable
  Scenario: By repo can be read: no label covers another label or a part
    When a person turns on By repo, in the structure, data flow or control flow lens
    Then no seam chip, flow label or bundle badge overlaps another or a part card
    And every part's name is whole

  @map.by-repo-bundles
  Scenario: The seams inside one repository fold into a bundle per pair of parts
    Given By repo is on
    Then the seams between two parts of one repository are one line with a badge that keeps any warning
    And the seams that leave a repository are drawn in full
    When the person points at a badge, or selects one of its parts
    Then the bundle opens into its seams, which can be selected and dropped on

  @map.by-repo-arrange
  Scenario: Parts move inside their repository's box, and a box moves with its parts, for everyone
    Given two people with By repo on
    When one of them drags a part inside its box, or a box by its name
    Then the other sees it in its new place, the box fitted to its parts
    And it is still there after a reload
    And a part dragged past the box's edge stays inside it

  @map.by-repo-stays-in-box
  Scenario: A part or a box dropped on another repository's box goes back
    Given a map with two repositories and By repo on
    When a person drops a part, or a whole box, on the other repository's box
    Then it goes back to where it was: moving code between repositories is a map draft

  @map.by-repo-flat-kept
  Scenario: Arranging By repo never moves the flat map
    Given parts and boxes moved with By repo on
    When the person turns By repo off
    Then every part is where the flat map had it

  @map.by-repo-auto-arrange
  Scenario: Auto-arrange in By repo clears only the By repo arrangement, with Undo
    Given By repo is on and the team has moved a box
    When a person presses Auto-arrange
    Then the By repo arrangement is laid out automatically for everyone and the flat map stays
    And Undo brings it back

  @map.flow-label-in-full
  Scenario: A flow label can be read in full on the map
    Given a flow label too long for the space between two parts
    When a person points at it or focuses it
    Then its whole text shows

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

  @map.traffic-first
  Scenario: A fresh traffic report is the first thing the map shows
    Given a fresh traffic report for the workspace
    When a person opens the map
    Then it shows the Traffic lens, headed by the calls a minute, the error share and what the map does not explain
    And each seam that carried calls is a pipe, heavier for more calls and coloured by its error share
    And each part says its calls a minute in and out, and a part with no calls steps back
    When the report is not fresh
    Then the map opens on Structure

  @map.ghosts-and-discrepancies
  Scenario: Observed traffic the map does not declare shows as ghosts and discrepancies
    Given a traffic report with calls to services the map does not have
    When a person picks the Traffic lens
    Then the unknown services show as ghosts in their own band
    And the declared parts stay where they were
    And the map-versus-territory list counts each discrepancy

  @map.empty-report-says-so
  Scenario: A traffic report with nothing in it is not read as the map matching
    Given a traffic source that answers with no calls between services
    When a person picks the Traffic lens
    Then the map-versus-territory list says nothing was observed
    And it does not say the observed system matches the map

  @map.dismiss-discrepancy
  Scenario: A person dismisses a discrepancy for the workspace
    Given a discrepancy between the map and the observed traffic
    When a person dismisses it
    Then it leaves the list for everyone in the workspace
    And the map file is not changed
