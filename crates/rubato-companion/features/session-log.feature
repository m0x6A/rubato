Feature: Prompts and replies of launched sessions, for the session log
  A session launched from Rubato forwards its prompt (UserPromptSubmit) and its last reply
  (Stop), each cut to Limits.session_log_max_chars (ADR 0052). A prompt that carries a session
  message the companion typed names that message, and says whether someone had typed before it
  (mixed). Every other session's prompts and replies stay on the machine (hooks.feature).

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin

  Scenario: The prompt that carried a delivered message names it
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the message:
      """
      line one
      line two
      line three
      """
    Then the server receives the outcome "delivered" for that message
    When the launched session submits that message as its prompt
    Then the server receives the prompt with that message's id, not mixed

  Scenario: A prompt with half-typed text before the message is mixed
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the message "Reply with exactly PONG."
    Then the server receives the outcome "delivered" for that message
    When the launched session submits "half-typed " and then that message as its prompt
    Then the server receives the prompt with that message's id, mixed

  Scenario: A prompt no message carried is forwarded without a message id
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session submits "typed by hand in the pane" as its prompt
    Then the server receives the prompt "typed by hand in the pane" with no message id

  Scenario: A long reply is cut to the limit and marked truncated
    Given the server limits session log text to 20 characters
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session stops with a reply of 50 characters
    Then the server receives the reply cut to 20 characters, marked truncated
