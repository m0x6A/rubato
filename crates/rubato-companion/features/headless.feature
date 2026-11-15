Feature: A headless companion runs each turn as a claude -p child (ADR 0062)
  An agent-only client is a companion in headless mode: no tmux, no tmux-ide. Each turn is one
  `claude -p` child that the companion holds. The child's exit and stream-json `result` end the
  turn and carry the reply. The stand-in claude here posts no hooks, so nothing below depends
  on them. Every scenario runs with tmux and tmux-ide unusable.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion is in headless mode with tmux and tmux-ide unusable

  Scenario: A headless launch runs the first turn and reports its reply
    Given the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And the headless stand-in ran 1 turn, the first with the launch's prompt and session id
    And the server receives a hook event "Stop" for the launched session with the reply "PONG"
    And the server receives a sessions snapshot for the launched session from source "headless" with status "idle"

  Scenario: A message is one resume turn, and a message during a turn waits
    Given the headless stand-in takes 2 seconds per turn
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A" and its first turn ended
    When the server sends the launched session the message "first"
    Then the server receives the outcome "delivered" for that message
    When the server sends the launched session the message "second"
    Then the server receives the outcome "not_idle" for that message
    And the headless stand-in ran 2 turns, the last resuming the session with "first"

  Scenario: Stop kills the running turn and ends the session
    Given the headless stand-in takes 30 seconds per turn
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    When the owner stops the launched session
    Then the server receives the outcome "delivered" for that message
    And no headless stand-in turn is running
    And the server receives a sessions snapshot for the launched session from source "headless" with status "ended"

  Scenario: A revoked pairing stops every headless turn
    Given the headless stand-in takes 30 seconds per turn
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    When the server says the pairing was revoked
    Then no headless stand-in turn is running

  Scenario: A turn the previous run left running is killed before anything starts
    With the reap at start removed, the planted turn is still running and this fails.
    Given a headless turn the previous run left running
    And the companion daemon is running and connected
    Then the planted turn is no longer running
