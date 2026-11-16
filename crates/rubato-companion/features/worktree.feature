Feature: One worktree and one tmux session per issue
  A launch for the paired user creates the worktree with git at <repo>-worktrees/<branch> (the
  path tmux-ide used, ADR 0070), starts Claude Code in a new tmux session with the session id
  from the command, the hooks injected with --settings, and the OTel resource attributes set.

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
    And the launch directory has mode 0700 and its prompt file mode 0600
    And that settings file holds no token and names a 0600 header file beside it
    And the stand-in claude received an MCP config, mode 0600, beside its settings file
    And the server receives a sessions snapshot with issue 7 of "acme/pantry" from source "launch"

  Scenario: A second launch for the same issue is refused as already running
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "refused" with reason "already_running"

  Scenario: A new branch gets no upstream until its first push
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And a worktree exists for branch "issue/7-restock" of "acme/pantry" beside the clone
    And the worktree for branch "issue/7-restock" of "acme/pantry" has no upstream

  Scenario: An existing local branch gets its worktree
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the clone of "acme/pantry" has a local branch "issue/7-restock"
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And a worktree exists for branch "issue/7-restock" of "acme/pantry" beside the clone
    And a tmux session exists for "acme/pantry" branch "issue/7-restock" with a pane in that worktree

  Scenario: A branch pushed from elsewhere is tracked from origin
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And "acme/pantry" has a branch "issue/7-restock" on origin only
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And a worktree exists for branch "issue/7-restock" of "acme/pantry" beside the clone
    And the worktree for branch "issue/7-restock" of "acme/pantry" tracks "origin/issue/7-restock"

  Scenario: A second launch after the session ended reuses the worktree
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion's git records its calls
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the tmux session for "acme/pantry" branch "issue/7-restock" ends
    And the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And a worktree exists for branch "issue/7-restock" of "acme/pantry" beside the clone
    And the companion's git ran "worktree add" 1 time

  Scenario: A leftover directory at the worktree path is refused and kept
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a leftover directory at the worktree path for branch "issue/7-restock" of "acme/pantry"
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a failed launch result whose error contains "exists but is not a worktree"
    And the leftover directory for branch "issue/7-restock" of "acme/pantry" is as it was
    And no tmux session exists for "acme/pantry" branch "issue/7-restock"

  Scenario: A stale registration for the branch is removed, and only that one
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a stale worktree registration for branch "issue/7-restock" of "acme/pantry"
    And a stale worktree registration for branch "issue/8-menu" of "acme/pantry"
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And a worktree exists for branch "issue/7-restock" of "acme/pantry" beside the clone
    And the stale worktree registration for branch "issue/8-menu" of "acme/pantry" is still listed

  Scenario: A tmux below the pinned version is refused
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a tmux that reports version "3.5"
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "refused" with reason "missing_dependency"
    And the refusal's detail contains "3.5" and "3.7"
    And no worktree exists for branch "issue/7-restock" of "acme/pantry"
