Feature: Mirroring a launched session's plan as a draft
  At a launched session's settle points (Stop, idle_prompt, SessionEnd) the companion reads the
  plan at the launch's plan path and the files it links (bundle_paths), and sends the bundle to
  the server when its hash changed (ADR 0053). Nothing is mirrored mid-turn, for a session not
  launched from Rubato, or to a server that did not announce drafts.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin

  Scenario: A plan and its linked design note are sent at Stop
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    And the launched session's worktree has "docs/plans/7.md":
      """
      # Restock

      The design is in [the note](7-restock/design.md).
      """
    And the launched session's worktree has "docs/plans/7-restock/design.md":
      """
      # Design, first cut
      """
    When the launched session posts a "Stop" hook
    Then the server receives 1 draft bundle
    And the last draft bundle has the files "docs/plans/7.md, docs/plans/7-restock/design.md"
    And the last draft bundle's hash matches its files

  Scenario: Two Stops with nothing changed send one bundle
    Remove the hash check and the second Stop sends the same bundle again, so this fails.
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    And the launched session's worktree has "docs/plans/7.md":
      """
      # Restock

      The design is in [the note](7-restock/design.md).
      """
    And the launched session's worktree has "docs/plans/7-restock/design.md":
      """
      # Design, first cut
      """
    When the launched session posts a "Stop" hook
    Then the server receives 1 draft bundle
    When the launched session posts a "Stop" hook
    Then the companion logs "draft bundle unchanged"
    And the server has received 1 draft bundle

  Scenario: Editing the design note sends a second bundle
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    And the launched session's worktree has "docs/plans/7.md":
      """
      # Restock

      The design is in [the note](7-restock/design.md).
      """
    And the launched session's worktree has "docs/plans/7-restock/design.md":
      """
      # Design, first cut
      """
    When the launched session posts a "Stop" hook
    Then the server receives 1 draft bundle
    When the launched session's worktree now has "docs/plans/7-restock/design.md":
      """
      # Design, second cut
      """
    And the launched session posts a "Stop" hook
    Then the server receives 2 draft bundles
    And the last two draft bundles have different hashes

  Scenario: A file the plan links that does not exist is listed as missing
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    And the launched session's worktree has "docs/plans/7.md":
      """
      # Restock

      See [the note](7-restock/design.md) and [the sketch](7-restock/sketch.md).
      """
    And the launched session's worktree has "docs/plans/7-restock/design.md":
      """
      # Design
      """
    When the launched session posts a "Stop" hook
    Then the server receives 1 draft bundle
    And the last draft bundle has the files "docs/plans/7.md, docs/plans/7-restock/design.md"
    And the last draft bundle lists "docs/plans/7-restock/sketch.md" as missing

  Scenario: A bundle over the limit is sent refused, with no files
    Given the server limits drafts to 100 bytes
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    And the launched session's worktree has "docs/plans/7.md":
      """
      # Restock

      The design is in [the note](7-restock/design.md).
      """
    And the launched session's worktree has "docs/plans/7-restock/design.md" of 500 characters
    When the launched session posts a "Stop" hook
    Then the server receives 1 draft bundle
    And the last draft bundle is refused as too large, with no files and no hash

  Scenario: A session not launched from Rubato is never mirrored
    Given the companion daemon is running and connected
    And the clone of "acme/pantry" has "docs/plans/7.md":
      """
      # Restock, planned by hand
      """
    When a session started by hand in the clone of "acme/pantry" posts a "Stop" hook
    Then no draft bundle arrives within 2 seconds

  Scenario: A server whose Welcome did not announce drafts gets no bundle
    Given the server does not announce drafts
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    And the launched session's worktree has "docs/plans/7.md":
      """
      # Restock
      """
    When the launched session posts a "Stop" hook
    Then the companion logs "draft bundle not sent: the server did not announce drafts"
    And no draft bundle arrives within 1 second
