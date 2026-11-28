Feature: Workspaces
  One server can hold several applications, each with its own map (ADR 0044). The page shows one
  workspace at a time, and the workspace is in the URL.

  @workspaces.switch
  Scenario: Switching workspaces changes the map, the URL and what a reload shows
    Given a server with the workspaces "default" and "shop"
    When a person picks "shop" in the workspace menu
    Then the map shows the shop's parts
    And the URL names the workspace
    And a reload shows the shop again

  @workspaces.unknown-falls-back
  Scenario: A link to a workspace the server does not hold falls back to the default
    When a person opens a link to a workspace the server does not hold
    Then they see the default workspace
    And they are told the workspace they asked for does not exist

  @workspaces.layout-per-workspace
  Scenario: Where parts sit is kept per workspace
    Given a part moved in the "shop" workspace
    When a person opens the "default" workspace
    Then that part is where it was in "default"
