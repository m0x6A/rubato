Feature: Restoring a parked draft into a new worktree
  A launch that resumes a parked draft carries the draft's newest revision (ADR 0053, plan 122
  §D5). The companion writes it only into a worktree the launch created, before the session
  starts, and says so in the Started result. A reused worktree is never written: when its files
  equal the draft it counts as restored, otherwise it is left alone. A path that leaves the
  worktree writes nothing at all. In the tables, \n is a newline.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin

  Scenario: A fresh worktree gets the draft before the session starts
    Given the companion daemon is running and connected
    Then the server receives a hello announcing "draft_restore"
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock" and the draft:
      | path                           | text                                           |
      | docs/plans/7.md                | # Restock\n\nSee [the note](7-restock/design.md).\n |
      | docs/plans/7-restock/design.md | # Design, parked\n                             |
    Then the server receives a launch result "started" with the draft restored
    And the launched worktree has "docs/plans/7.md" reading "# Restock\n\nSee [the note](7-restock/design.md).\n"
    And the launched worktree has "docs/plans/7-restock/design.md" reading "# Design, parked\n"
    When the launched session posts a "Stop" hook
    Then the companion logs "draft bundle unchanged"
    And the server has received 0 draft bundles

  Scenario: A reused worktree whose files differ is left alone
    Only a worktree this launch created is written. Write into a reused one and this fails.
    Given a worktree for branch "issue/7-restock" of "acme/pantry" already has "docs/plans/7.md":
      """
      # Restock, edited by hand
      """
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock" and the draft:
      | path            | text                  |
      | docs/plans/7.md | # Restock, parked\n   |
    Then the server receives a launch result "started" with the restore skipped as "worktree_differs"
    And the launched worktree has "docs/plans/7.md" reading "# Restock, edited by hand\n"

  Scenario: A reused worktree that already holds the draft counts as restored
    Given a worktree for branch "issue/7-restock" of "acme/pantry" already has "docs/plans/7.md":
      """
      # Restock, parked
      """
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock" and the draft:
      | path            | text                |
      | docs/plans/7.md | # Restock, parked\n |
    Then the server receives a launch result "started" with the draft restored

  Scenario Outline: A path that leaves the worktree writes nothing
    Given the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock" and the draft:
      | path            | text                |
      | docs/plans/7.md | # Restock, parked\n |
      | <path>          | escaped\n           |
    Then the server receives a launch result "started" with the restore skipped as "path_outside"
    And the launched worktree has no "docs/plans/7.md"
    And nothing was written at "<path>" outside the launched worktree

    Examples:
      | path                    |
      | ../escaped.md           |
      | <root>/abs-escaped.md   |

  Scenario: A command from an older server, with no draft, changes nothing
    Given the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started" with no draft restored
    And the launched worktree has no "docs/plans/7.md"
