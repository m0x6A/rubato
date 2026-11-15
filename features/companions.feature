Feature: Companions
  Each developer runs one companion on their own machine. It connects outbound to the server and
  pairs with its owner's login through a device code.

  @companions.pair-with-code
  Scenario: A companion pairs with a code its owner approves
    Given a companion on a developer's machine that shows a pairing code
    When its owner approves the code in Rubato
    Then the companion is paired with that owner
    And it connects and shows as connected

  @companions.see-teammates
  Scenario: Everyone sees each teammate's companion and its lane, with no controls on it
    Given a teammate's companion is connected
    When a person opens Companions
    Then they see the teammate's companion as connected, with its queue
    And they have no controls on it

  @companions.revoke
  Scenario: An owner revokes their companion's pairing
    Given a person's paired companion is connected
    When they revoke the pairing
    Then the companion is disconnected
    And it cannot connect again without pairing

  @companions.headless-badge
  Scenario: A headless companion shows as headless
    Given a person paired a headless companion
    When anyone opens Companions
    Then that companion is marked headless
