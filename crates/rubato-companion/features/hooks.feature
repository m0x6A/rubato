Feature: Forwarding Claude Code hook events
  Claude Code posts each hook's stdin JSON to the companion over loopback with a bearer token
  (ADR 0013), which the hook command reads from a 0600 header file, never from its own argv
  (ADR 0063). Each "posts" step below runs the rendered hook command under sh, as Claude Code
  does. The companion forwards the event and derives the session status.

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

  Scenario: A forwarded hook travels with the trace context of its send
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "stop.json" to the companion
    Then the server receives a hook event "Stop" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the envelope of that hook event carries a W3C traceparent of its own

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

  Scenario: The hook command reads the token from a private file, not from its argv
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "stop.json" to the companion
    Then the server receives a hook event "Stop" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the hook header file is private and no hook command holds the token

  Scenario: A hook naming /dev/zero as its transcript is refused, not read
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion prices models starting with "claude-"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "stop.json" naming "/dev/zero" as its transcript
    Then the server receives a hook event "Stop" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the companion logs "rubato.transcript.refused"
    And the companion daemon is still running

  Scenario: Tool input and output never leave the machine
    Only an allowlist of payload keys is forwarded (ADR 0052). The fixture adds a key no
    allowlist knows, so a filter that only drops known secrets fails here.
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "post-tool-use-extra.json" to the companion
    Then the server receives a hook event "PostToolUse" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the forwarded hook payload has no key "tool_input"
    And the forwarded hook payload has no key "tool_response"
    And the forwarded hook payload has no key "secret_x"
    And the forwarded hook payload keeps key "tool_name"

  Scenario Outline: A prompt or reply from a session not launched from Rubato never leaves the machine
    Only a launched session feeds the session log (ADR 0052, plan 122 PR 4).
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "<fixture>" to the companion
    Then the server receives a hook event "<event>" for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the forwarded hook payload has no key "<key>"
    And the forwarded hook names no launch

    Examples:
      | fixture                 | event            | key                    |
      | user-prompt-submit.json | UserPromptSubmit | prompt                 |
      | stop.json               | Stop             | last_assistant_message |
