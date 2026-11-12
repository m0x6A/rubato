Feature: A launch runs on its own task, and its external commands have a deadline
  Launch commands are handled one at a time on the launch task, not in the loop that forwards
  hooks, so a slow launch does not hold hook events back. Every git, tmux-ide and tmux call has a
  deadline; a launch whose command hits it is reported as failed (plan 40).

  Scenario: Hooks are forwarded while a launch is inside tmux-ide
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a tmux-ide that hangs on "worktree create"
    And a command timeout of 5 seconds
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    And the launch is inside the hanging tmux-ide
    And Claude Code posts the recorded hook "user-prompt-submit.json" to the companion
    Then the server receives a hook event "UserPromptSubmit" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the server receives a failed launch result whose error contains "timed out after 5s"
    And the hook event reached the server before the launch result

  Scenario: A launch whose tmux-ide hangs fails at the deadline and leaves no session
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a tmux-ide that hangs on "worktree create"
    And a command timeout of 5 seconds
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a failed launch result whose error contains "tmux-ide worktree create"
    And the companion logs "external command timed out"
    And no tmux session exists for "acme/pantry" branch "issue/7-restock"
    And the hanging tmux-ide is no longer running
