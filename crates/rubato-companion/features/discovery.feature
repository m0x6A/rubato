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
