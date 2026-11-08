Feature: Refusing a launch for another user
  A user starts sessions only in their own tmux (ADR 0004). A command addressed to anyone else
  is refused, reported, and leaves no trace on the machine.

  Scenario: A launch addressed to another user is refused and does nothing
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "alice" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "refused" with reason "not_paired_user"
    And no worktree exists for branch "issue/7-restock" of "acme/pantry"
    And no tmux session exists for "acme/pantry" branch "issue/7-restock"
