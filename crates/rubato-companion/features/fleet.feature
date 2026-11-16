Feature: A person's lead agent asks its own companion for its fleet
  With the companion's orchestrator flag on (ADR 0072, plan 134), a lead launch gets four fleet
  tools on its MCP server, behind a secret only its own mcp.json holds. A call becomes one
  FleetRequest for that launch, and the server's FleetResult answers it. With the flag off the
  companion announces nothing and refuses a lead launch.

  Scenario: A lead's fleet call reaches the server as its own launch and gets the answer
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion's orchestrator flag is on
    And the test server also announces "fleet"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    Then the server receives a hello announcing "fleet"
    When the server sends a lead launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "lead"
    Then the server receives a launch result "started"
    And the lead's MCP config carries its lead secret, and a worker's would not
    When the lead asks fleet_message to a worker through its MCP config and the server answers "ok"
    Then the fleet request named the lead's own launch

  Scenario: With the flag off a lead launch is refused
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a lead launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "lead"
    Then the server receives a launch result "refused" with reason "orchestrator_off"
