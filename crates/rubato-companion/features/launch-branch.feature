Feature: A launch runs only on the issue's own branch, with every server value as text
  The branch names the worktree folder, goes to tmux-ide and into the launch script, so the
  companion launches only on `issue/<n>-<slug>` for the command's own issue (ADR 0006, #155).
  Values from the server, such as a map node id, stay text in the launch script.

  Scenario: A launch on another issue's branch fails and leaves nothing
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" on branch "issue/9-other"
    Then the server receives a failed launch result whose error contains "is not issue/7-<slug> for acme/pantry#7"
    And no worktree exists for branch "issue/9-other" of "acme/pantry"
    And no tmux session exists for "acme/pantry" branch "issue/9-other"

  Scenario: A slug that climbs out of the worktree folder fails and leaves nothing
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "../../escape"
    Then the server receives a failed launch result whose error contains "is not issue/7-<slug>"
    And no worktree exists for branch "issue/7-../../escape" of "acme/pantry"
    And no tmux session exists for "acme/pantry" branch "issue/7-../../escape"

  Scenario: A node id with a quote reaches Claude Code as text and runs nothing
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock" and node "a'$(touch${IFS}pwned)'"
    Then the server receives a launch result "started"
    And the stand-in claude received "rubato.node=a'$(touch${IFS}pwned)'" in OTEL_RESOURCE_ATTRIBUTES
    And no file "pwned" exists in the worktree or the clone
