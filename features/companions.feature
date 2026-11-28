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

  @companions.agent-line
  Scenario: A launched session shows what its agent says it is doing, with its age
    Given a person launched a session through their companion
    And its agent reported a line of what it is doing through MCP
    When anyone opens Companions or the issue
    Then the line is shown under that session with how long ago it was said
    And it is never shown as a state, and never under another companion's session

  @companions.pairing-steps
  Scenario: Pairing says how, step by step
    Given a person with no companion connected
    When they open an issue
    Then the issue says a companion starts sessions, with a way to pair one
    When they follow it
    Then the pairing view shows the command to run with this Rubato's address, where the code goes, and how to start the companion
    And someone who has a companion already sees their lanes, with pairing one step away
