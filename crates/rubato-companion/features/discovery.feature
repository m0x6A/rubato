Feature: Discovering sessions
  Identity comes from the branch: pane cwd → worktree → branch → issue (ADR 0006). State comes
  from hooks only; a pane is metadata, so anything without a hook is unknown (ADR 0070).

  Scenario: A pane in an issue worktree is discovered, untrusted until a hook speaks
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a worktree for branch "issue/9-menu" of "acme/pantry" with a tmux pane running the stand-in claude in it
    And the companion daemon is running and connected
    Then the server receives a sessions snapshot with issue 9 of "acme/pantry" on branch "issue/9-menu"
    And that snapshot entry has source "scraped" and status "unknown"

  Scenario: A hook from the pane's session makes it trusted
    A session started by hand reports state once its hooks are opted in (plan 147 Q3).
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a worktree for branch "issue/9-menu" of "acme/pantry" with a tmux pane running the stand-in claude in it
    And the companion daemon is running and connected
    Then the server receives a sessions snapshot with issue 9 of "acme/pantry" on branch "issue/9-menu"
    When Claude Code in the worktree for branch "issue/9-menu" of "acme/pantry" posts the recorded hook "user-prompt-submit.json"
    Then the server receives a sessions snapshot where issue 9 of "acme/pantry" has source "hook" and status "working"

  Scenario: A session started by hand reports through the hooks render-hooks wrote into its worktree
    ADR 0078: the opt-in the companion merges into .claude/settings.local.json is what such a
    session runs; its command posts to the companion like a launched session's.
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And a worktree for branch "issue/9-menu" of "acme/pantry" with a tmux pane running the stand-in claude in it
    And the companion daemon is running and connected
    And the developer opts the worktree for branch "issue/9-menu" of "acme/pantry" in by hand
    Then the server receives a sessions snapshot with issue 9 of "acme/pantry" on branch "issue/9-menu"
    When Claude Code started by hand in the worktree for branch "issue/9-menu" of "acme/pantry" fires the opted-in hook "user-prompt-submit.json"
    Then the server receives a sessions snapshot where issue 9 of "acme/pantry" has source "hook" and status "working"

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
