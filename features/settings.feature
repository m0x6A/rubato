Feature: Workspace settings
  A workspace's name, map ref, traffic source and launch defaults are set at runtime from
  Configure workspace (ADR 0075). Application settings are not there, and a secret shows only
  as set or not set.

  @settings.rename
  Scenario: A new name shows in the switcher without a reload
    Given a server with the workspaces "default" and "shop"
    When a person opens Configure workspace in "shop" and saves a new name
    Then the switcher shows the new name
    And the page was not reloaded

  @settings.move-map-ref
  Scenario: Moving the map's ref loads the map at that ref
    Given the workspace "default" reads its map at "main"
    When a person saves the ref of another branch that holds the map
    Then the header shows the map at that branch's commit
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
    When a member opens Configure workspace from the header
    Then they see that workspace's settings
    And no switcher is drawn and no workspace is created
