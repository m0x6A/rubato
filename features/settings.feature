Feature: Workspace settings
  A workspace's name, map ref, traffic source and launch defaults are set at runtime from
  Configure workspace (ADR 0075). Application settings are not there, and a secret shows only
  as set or not set.

  @settings.rename
  Scenario: A new name shows in the workspace menu without a reload
    Given a server with the workspaces "default" and "shop"
    When a person opens Configure workspace in "shop" and saves a new name
    Then the workspace menu shows the new name
    And the page was not reloaded

  @settings.move-map-ref
  Scenario: Moving the map's ref loads the map at that ref
    Given the workspace "default" reads its map at "main"
    When a person saves the ref of another branch that holds the map
    Then the workspace menu shows the map at that branch's commit
    And the page was not reloaded

  @settings.refusal-explained
  Scenario: A refused change says why and changes nothing
    Given a workspace whose map is a file on the server
    When a person saves another ref for it
    Then they are told the ref cannot move
    And the ref is as it was

  @settings.single-workspace
  Scenario: A server with one workspace still offers Configure workspace
    Given a server with one workspace
    When a member opens Configure workspace from the workspace menu
    Then they see that workspace's settings
    And the menu offers no other workspace and no workspace is created

  @settings.traffic-per-workspace
  Scenario: Each workspace has its own traffic source
    Given a server with the workspaces "default" and "shop", neither with a traffic source
    When a person sets a traffic source in Configure workspace for "shop"
    Then "shop" shows that source
    And "default" still has none, and its Traffic lens says where to set one

  @settings.traffic-status
  Scenario: Configure workspace says how the workspace's traffic source is doing
    Given the workspace "shop" has a traffic source that cannot be reached
    When a person opens Configure workspace in "shop"
    Then they read that the source is failing, with the reason
    And Configure workspace in "default" says nothing about a source

  @settings.traffic-namespaces
  Scenario: A workspace's traffic source reads only the namespaces it lists
    Given the workspace "shop" has a traffic source on a stack shared with other applications
    When a person lists the service namespaces to include and saves
    Then the source keeps the list, and reads only calls with an end in one of them
    And a list that is not namespaces is refused with the reason
