Feature: Pairing a companion with a GitHub login
  The companion connects outbound only (ADR 0004) and authenticates with an opaque pairing
  token from the device-code flow (ADR 0011).

  Scenario: Pairing stores the token privately
    Given a test server that approves pairing for user "m0x6A"
    When the companion pairs with the server
    Then the state file holds the pairing token with mode 0600
    And the state file names user "m0x6A"

  Scenario: An unpaired companion refuses to run
    Given no state file
    When the companion is asked to run
    Then it fails with an error mentioning "pair"

  Scenario: Hello carries the pairing identity
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    When the companion daemon starts
    Then the server receives a hello for user "m0x6A" with protocol version 1
    And the server receives a heartbeat naming the tmux-ide version

  Scenario: Shutdown stops every task and returns Ok
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When the companion is told to shut down
    Then the daemon returns Ok within 1 second
    And the companion logs no "rubato.task.exited" event
