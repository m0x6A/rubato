Feature: Pinning issues to the map
  An issue is pinned to a part or a seam, never only to a repository.

  @pins.drag-onto-part
  Scenario: Dragging a not-pinned issue onto a part pins it there
    Given an open issue nobody has pinned
    When a person drags it from the backlog onto a part
    Then the issue is Planned and pinned to that part
    And it leaves the backlog
    And the part counts it

  @pins.drag-onto-seam
  Scenario: Dragging a not-pinned issue onto a seam pins it there
    Given an open issue nobody has pinned
    When a person drags it onto a seam
    Then the issue is Planned and pinned to that seam
    And the seam counts it

  @pins.map-line-member
  Scenario: A team member's Map: line places the issue without a drag
    Given an issue opened by a team member whose body names a seam on a Map: line
    When Rubato reads it
    Then the issue is Planned on that seam

  @pins.map-line-stranger-suggests
  Scenario: A Map: line from someone outside the team only suggests
    Given an issue opened by someone outside the team whose body names a part on a Map: line
    When Rubato reads it
    Then the issue stays in the backlog
    And it shows the part as a suggestion from someone outside the team

  @pins.unpin
  Scenario: Unpinning an issue sends it back to the backlog
    Given an issue pinned to a part
    When a person unpins it
    Then it is in the backlog again
    And the part no longer counts it
