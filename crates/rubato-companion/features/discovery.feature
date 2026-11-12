Feature: Discovering sessions
  Identity comes from the branch: pane cwd → worktree → branch → issue (ADR 0006). State comes
  from hooks or from tmux-ide with hook authority; anything else is unknown.

  Scenario: A pane in an issue worktree is discovered, untrusted until a hook speaks
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a worktree for branch "issue/9-menu" of "acme/pantry" with a tmux pane running the stand-in claude in it
    And the companion daemon is running and connected
    Then the server receives a sessions snapshot with issue 9 of "acme/pantry" on branch "issue/9-menu"
    And that snapshot entry has source "scraped" and status "unknown"

  Scenario: tmux-ide team --json from the real tool parses
    Given a private tmux server
    When the companion runs tmux-ide team --json
    Then it parses and the tmux-ide version is at least "2.9.0"

  Scenario: A recorded fleet with hook authority yields a trusted state
    Given the recorded tmux-ide fleet "team.json"
    When the companion merges it into an empty registry
    Then pane "%2" has status "unknown" from source "scraped"
    And pane "%5" has status "working" from source "tmux_ide"

  Scenario: With no tmux-ide events to follow, discovery runs on the timer and the companion stays up
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And tmux-ide has no events to follow
    And a temporary repository "acme/pantry" with a bare origin
    And a worktree for branch "issue/9-menu" of "acme/pantry" with a tmux pane running the stand-in claude in it
    And the companion daemon is running and connected
    Then the server receives a sessions snapshot with issue 9 of "acme/pantry" on branch "issue/9-menu"
    And the companion logs "tmux-ide events unavailable; timer only"
    When 3 seconds pass
    Then the companion daemon is still running

  Scenario: A session that checks out an issue branch moves onto that issue
    Issue #80: identity is read again when the worktree's HEAD changes, off the registry lock.
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a tmux pane running the stand-in claude in the clone of "acme/pantry"
    And the companion daemon is running and connected
    Then the server receives a sessions snapshot with a session on branch "main" and no issue
    When the clone of "acme/pantry" checks out a new branch "issue/42-menu"
    Then the server receives a sessions snapshot with issue 42 of "acme/pantry" on branch "issue/42-menu"

  Scenario: Claude Code started by a wrapper script is found
    Issue #80: the pane's command is the wrapper's shell; its process tree runs claude.
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a worktree for branch "issue/9-menu" of "acme/pantry" with a tmux pane running the stand-in claude under a wrapper script
    And the companion daemon is running and connected
    Then the server receives a sessions snapshot with issue 9 of "acme/pantry" on branch "issue/9-menu"
