Feature: Session messages typed into your own launched session
  The server sends a SessionMessage for a session its paired user launched (ADR 0051). The
  companion types it through tmux, once, and only when a hook says the session waits for input
  and that hook is newer than the companion's own last input. It answers each message with one
  SessionMessageResult and never reads the pane: the stand-in claude records what it was typed.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"

  Scenario: A three-line message is typed exactly once after a Stop hook
    Given a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the message:
      """
      line one
      line two
      line three
      """
    Then the server receives the outcome "delivered" for that message
    And the stand-in claude was typed exactly:
      """
      line one
      line two
      line three
      """

  Scenario: A message while the session works is not typed
    Given a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "UserPromptSubmit" hook
    And the server sends the launched session the message "are you there?"
    Then the server receives the outcome "not_idle" for that message
    And nothing was typed into the stand-in claude

  Scenario: A second message waits for the next Stop even though the session still shows idle
    Only a hook newer than the last paste proves the turn it began has ended. Remove
    `input_sent_at` and the second message is typed at once, so this scenario fails.
    Given a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the message "first"
    Then the server receives the outcome "delivered" for that message
    When the server sends the launched session the message "second"
    Then the server receives the outcome "not_idle" for that message
    And the companion still reports the launched session as "idle"
    And the stand-in claude was typed exactly:
      """
      first
      """
    When the launched session posts a "Stop" hook
    And the server sends that message again
    Then the server receives the outcome "delivered" for that message
    And the stand-in claude was typed exactly:
      """
      first
      second
      """

  Scenario: A message for another user is refused
    Given the companion daemon is running and connected
    When the server sends a message for user "someone-else" to a launch
    Then the server receives the refusal "not_paired_user" for that message

  Scenario: A message for a session this companion did not launch is refused
    Given the companion daemon is running and connected
    When the server sends a message for user "m0x6A" to a launch
    Then the server receives the refusal "not_launched_here" for that message

  Scenario: A delivered message pushed again is answered without typing
    The server pushes an unanswered message again at each Hello; deliveries.json answers it.
    Given a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the message "hello"
    Then the server receives the outcome "delivered" for that message
    When the launched session posts a "Stop" hook
    And the server sends that message again
    Then the server receives the outcome "delivered" for that message 2 times
    And the stand-in claude was typed exactly:
      """
      hello
      """

  Scenario: A message the companion was typing when it stopped is answered interrupted
    Given a temporary repository "acme/pantry" with a bare origin
    And the companion's delivery record says a message was being typed
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends that message to the launched session
    Then the server receives the outcome "interrupted" for that message
    And nothing was typed into the stand-in claude

  Scenario: A hook from a new session id with the launch header keeps the launch
    A /clear gives the session a new id. Its hooks send X-Rubato-Launch, so the launch moves to
    the new id instead of a second, unlaunched session appearing, and messages follow it.
    Given a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "SessionStart" hook as a new session id
    Then the companion reports the launch under the new session id and no other session
    And the server receives a sessions snapshot with the launch under the new session id
    And launches.json names the new session id
    When the launched session posts a "Stop" hook
    And the server sends the launched session the message "after the clear"
    Then the server receives the outcome "delivered" for that message
