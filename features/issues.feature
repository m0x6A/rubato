Feature: Issues
  GitHub is the source: Rubato lists the issues that are open there, and keeps the finished ones
  where a person can ask for them.

  @issues.open-only
  Scenario: An issue closed on GitHub leaves the default lists
    Given a tracked issue with a plan
    When the issue is closed on GitHub without a merged pull request
    Then the issue is not in the Issues list, and the Issues count does not include it
    And its plan is not among the proposed or open plans
    And the map shows it on no part

  @issues.done-view
  Scenario: Finished issues are listed under Done
    Given an issue closed on GitHub and an issue whose pull request merged
    When a person switches the Issues list to Done
    Then both are listed, the closed one marked closed
    And a finished issue's detail offers nothing that starts work

  @issues.listed-once
  Scenario: An issue with a plan but no pin is listed once
    Given an open issue nobody pinned, whose plan Rubato found by its branch
    When a person opens Issues
    Then the issue is listed with the tracked issues as not pinned
    And Not pinned does not list it a second time
    And its repository counts it once

  @issues.three-lists
  Scenario: The issue list is three lists, one at a time
    Given tracked issues, open issues nobody pinned, and finished issues
    When a person opens Issues
    Then they see the open tracked issues, and the counts of Not pinned and Done
    When they choose Not pinned
    Then they see the open issues nobody pinned, each with Pin to…, and the address names that list
    And an issue's other actions show when its row is pointed at or focused

  @issues.next-action
  Scenario: An issue's header offers the one next step
    Given an open issue
    When a person opens it
    Then the header says where the issue stands
    And it offers exactly one next step for that state: pin it, pair a companion, start or resume a session, open the session, review the plan, answer in the room, or review the pull request
    And a finished issue offers none

  @issues.parts
  Scenario: An issue's parts show one at a time
    Given an issue with a plan and a session
    When a person opens it
    Then they see its overview, and can choose its plan, its room, its sessions, and its pull request when it has one
    And what they typed in one part is still there when they come back to it
