Feature: Forwarding Claude Code hook events
  Claude Code posts each hook's stdin JSON to the companion over loopback with a bearer token
  (ADR 0013). The companion forwards it and derives the session status.

  Scenario Outline: A recorded hook event is forwarded and sets the status
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "<fixture>" to the companion
    Then the server receives a hook event "<event>" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the companion reports that session with status "<status>" from source "hook"
    And the companion reports that session blocked by "<blocked_by>"

    Examples:
      | fixture                        | event            | status  | blocked_by        |
      | session-start.json             | SessionStart     | working |                   |
      | user-prompt-submit.json        | UserPromptSubmit | working |                   |
      | pre-tool-use.json              | PreToolUse       | working |                   |
      | post-tool-use.json             | PostToolUse      | working |                   |
      | notification-permission.json   | Notification     | blocked | permission_prompt |
      | notification-idle.json         | Notification     | blocked | idle_prompt       |
      | stop.json                      | Stop             | idle    |                   |
      | session-end.json               | SessionEnd       | ended   |                   |

  Scenario: A working hook after a permission prompt clears the cause
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "notification-permission.json" to the companion
    Then the companion reports that session blocked by "permission_prompt"
    When Claude Code posts the recorded hook "pre-tool-use.json" to the companion
    Then the companion reports that session with status "working" from source "hook"
    And the companion reports that session blocked by ""

  Scenario: An unknown hook event is kept, not dropped
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "unknown-event.json" to the companion
    Then the server receives a hook event "Unknown" for session "0f8fad5b-d9cb-469f-a165-70867728950e"

  Scenario: A post without the token is refused and nothing is forwarded
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When something posts the recorded hook "stop.json" without the bearer token
    Then the companion answers 401
    And the server receives no hook event within 1 second
