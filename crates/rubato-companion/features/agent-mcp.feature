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

  Scenario: The Hello names agent_mcp
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    Then the server receives a hello announcing "agent_mcp"

  Scenario: A report reaches a server that takes agent_mcp
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the test server also announces "agent_mcp"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    When the launched session reports "designing the restock seam" on "rubato-companion" through its MCP config
    Then the server receives an agent report "designing the restock seam" for that launch

  Scenario: A server that does not take agent_mcp gets no report
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    When the launched session reports "designing the restock seam" on "rubato-companion" through its MCP config
    Then the companion logs "agent report not sent: server does not take agent_mcp"
    And the server has received no agent report

  Scenario: A plan status from the server answers plan_status, with the link up
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the test server also announces "agent_mcp"
    And a temporary repository "acme/pantry" with a bare origin
    And the companion daemon is running and connected
    When the server sends a launch for user "m0x6A" on issue 7 of "acme/pantry" with slug "restock"
    Then the server receives a launch result "started"
    When the server sends a plan status for that launch whose branch plan is "0f1e2d3"
    Then asked through that config, plan_status answers the branch plan "0f1e2d3" with the link up
