Feature: Starting a session
  A person starts a Claude Code session on an issue only through their own companion. Rubato
  never runs anything on a developer's machine itself.

  @launch.only-own-companion
  Scenario: Start appears only when the person's own companion is connected
    Given a pinned issue
    And only a teammate's companion is connected
    Then the person sees no Start button and is told they have no companion
    When their own companion connects
    Then Start appears
    And pressing it sends the launch to their own companion only

  @launch.session-runs-with-hooks
  Scenario: A launched session runs in a worktree with the hooks that report it
    Given a person's companion is paired and connected
    When they start a session on a pinned issue
    Then the companion starts Claude Code in a worktree on the issue's branch
    And the session's hooks report to the server, through to its Stop hook

  @launch.spend-cap-stops
  Scenario: A session that reaches its spend cap is stopped and the issue is Blocked
    Given a session launched with a spend cap from the plan
    When its spend reaches the cap
    Then the companion stops it
    And the issue shows Blocked, with the cap hit as the reason
