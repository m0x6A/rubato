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
    And the server receives a heartbeat naming the tmux version

  Scenario: Shutdown stops every task and returns Ok
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When the companion is told to shut down
    Then the daemon returns Ok within 1 second
    And the companion logs no "rubato.task.exited" event

  Scenario: A restarted companion is not mistaken for a replay
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    When the companion is shut down and started again
    Then the second hello has a sequence number above every one the server saw before it
    And the reservation in the state dir covers every sequence number the server saw

  # The pairing token and launch commands would cross the network in clear text (ADR 0067).
  Scenario: Pairing refuses a plain-http server off loopback
    Given no state file
    When the companion pairs with "http://rubato.example.test"
    Then it fails with an error mentioning "https"
    And no state file exists

  Scenario: A companion paired with a plain-http server off loopback refuses to start
    Given a companion paired with "http://rubato.example.test" for user "m0x6A"
    When the companion daemon starts
    Then the daemon returns an error mentioning "--insecure" within 5 seconds

  # The developer's explicit opt-in, kept in companion.json and warned about each time.
  Scenario: Pairing with --insecure to a plain-http LAN server succeeds and the daemon starts
    Given a test server on this machine's LAN address that approves pairing for user "m0x6A"
    When the companion pairs with the server using --insecure
    Then the state file allows plain http to the server
    And the companion has logged 1 warning mentioning "travel unencrypted"
    When the companion daemon starts
    Then the server receives a hello for user "m0x6A" with protocol version 1
    And the companion has logged 2 warnings mentioning "travel unencrypted"

  Scenario: status says which directories are opted in, and prints no token
    Sessions started by hand report only through a per-repo opt-in (ADR 0078); status is where a
    developer sees it, and its token stays in the header file (ADR 0063).
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/aster" with a bare origin
    And a temporary repository "acme/birch" with a bare origin
    And a temporary repository "acme/cedar" with a bare origin
    And the workspace is written to local.yml
    And the clone of "acme/cedar" has our hooks on port 1
    And the companion daemon is running and connected
    When the developer opts the clone of "acme/aster" in with render-hooks --into
    And the developer runs rubato-companion status
    Then status says "opted in" for the clone of "acme/aster"
    And status says "not opted in; run: rubato-companion render-hooks --into" for the clone of "acme/birch"
    And status says "opted in, stale port 1" for the clone of "acme/cedar"
    And status ends with "checked 3 directories: 1 opted in, 1 stale"
    And status says the hook endpoint is "listening"
    And status prints no token

  Scenario: status says when nothing listens on the hook port
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/aster" with a bare origin
    And the workspace is written to local.yml
    And the companion daemon is running and connected
    When the companion is told to shut down
    Then the daemon returns Ok within 5 seconds
    When the developer runs rubato-companion status
    Then status says the hook endpoint is "not listening"
