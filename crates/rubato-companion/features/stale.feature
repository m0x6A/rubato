Feature: A quiet session is unknown, never working
  Every signal carries a last-seen time. A session reporting working with no hook event for
  longer than the stale threshold renders unknown, and the companion says so once.

  Scenario: A working session that goes quiet is reported stale
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A" with a session stale threshold of 1 second
    And the companion daemon is running and connected
    When Claude Code posts the recorded hook "pre-tool-use.json" to the companion
    And 3 seconds pass
    Then the server receives a stale event for session "0f8fad5b-d9cb-469f-a165-70867728950e"
    And the companion reports that session with effective status "unknown"
    And the server receives exactly 1 stale event within 2 more seconds
