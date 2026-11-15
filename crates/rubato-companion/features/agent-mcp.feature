Feature: A launched session can ask its own companion over MCP
  Every session Rubato launches gets the companion's MCP server, rubato-companion, through
  --mcp-config (ADR 0071, plan 220 §5): a 0600 mcp.json beside hooks.json names the hook
  receiver's /mcp with its token and the launch id. The session asks what it was launched for
  with get_assignment, and the answer is its own launch.

  Scenario: The launched session's MCP config reaches the companion and names its launch
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    And the stand-in claude received an MCP config, mode 0600, beside its settings file
    And asked through that config, the MCP server answers get_assignment with issue 7 of "acme/pantry"
    And asked without the launch header, the MCP server refuses get_assignment
