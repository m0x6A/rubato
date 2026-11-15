Feature: Talking to your own session
  A person talks to the session they launched; the whole team can read the conversation.

  @chat.message-own-session
  Scenario: A message to your own session is typed into it once, when it waits for input
    Given a person's launched session waiting for input
    When they send it a message from Rubato
    Then the message is typed into the session once
    And the session's reply shows in the log for the team

  @chat.read-session-log
  Scenario: The team reads a launched session's prompts and last replies
    Given a launched session that has had prompts and replies
    When anyone opens its log
    Then they see the prompts and the last replies, in order

  @chat.stop-own-session
  Scenario: The owner stops their own session
    Given a person's launched session that is working
    When they press Stop
    Then the companion stops the session
    And nobody else can stop it
