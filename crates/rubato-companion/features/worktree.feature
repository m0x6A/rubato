Feature: One worktree and one tmux session per issue
  A launch for the paired user creates the worktree with tmux-ide, starts Claude Code in a new
  tmux session with the session id from the command, the hooks injected with --settings, and
  the OTel resource attributes set.

  Scenario: A launch creates the worktree and the session
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And a worktree exists for branch "issue/7-restock" of "acme/pantry" beside the clone
    And a tmux session exists for "acme/pantry" branch "issue/7-restock" with a pane in that worktree
    And the stand-in claude was started with the session id from the command
    And the stand-in claude received "rubato.issue=acme/pantry#7" in OTEL_RESOURCE_ATTRIBUTES
    And the stand-in claude received a settings file whose hooks post to the companion's port
    And the server receives a sessions snapshot with issue 7 of "acme/pantry" from source "launch"

  Scenario: A second launch for the same issue is refused as already running
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "refused" with reason "already_running"
