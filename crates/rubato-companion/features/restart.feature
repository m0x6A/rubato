Feature: Launched sessions survive a companion restart
  A companion upgrade restarts it while the sessions it launched keep running (plan 122 §D1).
  It keeps their launches in launches.json and binds the same hook port with the same token, so
  their hooks keep arriving and messages still reach them. When the port is taken it binds a
  new one, logs "hook port changed", and reports those launches as hooks_lost.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"

  Scenario: After a restart a session launched before posts to the same port with the same token
    When the companion restarts
    Then the hook endpoint has the same port and token as before the restart
    When the launched session posts a "Stop" hook
    Then the server receives a sessions snapshot with the launch, its hooks not lost
    When the server sends the launched session the message "still there?"
    Then the server receives the outcome "delivered" for that message
    And the stand-in claude was typed exactly:
      """
      still there?
      """

  Scenario: A restart that finds the hook port taken reports the launch's hooks as lost
    When the companion stops
    And something else takes the companion's hook port
    And the companion starts again
    Then the hook endpoint has a new port and a new token
    And the companion logs "hook port changed"
    And the server receives a sessions snapshot with the launch, its hooks lost
