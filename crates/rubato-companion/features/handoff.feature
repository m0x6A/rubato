Feature: The handoff after an approval in Rubato
  When a plan is approved in Rubato the server sends the launched session a plan_approved
  message. The companion types it as one plan.ts handoff command carrying the approval record
  (ADR 0055), and only when the installed rubato skill has that command. The skill reports what
  it did to the companion's loopback /handoff route, which forwards it to the server as a
  HandoffReport. launch.sh tells the session how it plans and where to report.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin

  Scenario: The Hello names the plan handoff and the installed skill
    Given the installed rubato skill is version "0.2.0"
    And the companion daemon is running and connected
    Then the server receives a hello announcing "plan_handoff"
    And the server receives a hello announcing "skill:0.2.0"

  Scenario: A skill that names no version is not announced
    Given the installed rubato skill names no version
    And the companion daemon is running and connected
    Then the server receives a hello announcing "plan_handoff"
    And the server receives a hello announcing nothing that starts with "skill:"

  Scenario: An approval is typed at idle as the handoff command
    The text typed, and the text a prompt is matched against, is the handoff command. Type the
    message's summary text instead and this scenario fails.
    Given the installed rubato skill is version "0.2.0"
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the approval of its plan
    Then the server receives the outcome "delivered" for that message
    And the stand-in claude was typed the handoff command for that approval
    When the launched session submits that message as its prompt
    Then the server receives the prompt with that message's id, not mixed

  Scenario: An approval for a session whose skill has no handoff is refused
    Given the installed rubato skill is version "0.1.0"
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session the approval of its plan
    Then the server receives the refusal "skill_outdated" for that message
    And nothing was typed into the stand-in claude

  Scenario: The launch tells the session how it plans and where to report
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A", planning in "rubato"
    Then the stand-in claude received "RUBATO_PLANNING=rubato" in its environment
    And the stand-in claude's environment points the handoff at the companion's hook endpoint

  Scenario: A handoff report posted to /handoff reaches the server
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    Then the stand-in claude received "RUBATO_PLANNING=git" in its environment
    When the launched session's skill reports a "committed" handoff to the companion
    Then the companion answers 202
    And the server receives a handoff report "committed" for the launch

  Scenario: A handoff report with the wrong token is refused
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session's skill reports a "committed" handoff with the token "not-the-token"
    Then the companion answers 401
    And no handoff report arrives within 1 second

  Scenario: A handoff post that is not a handoff line is refused
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session's skill posts this body to /handoff:
      """
      {"event": "rubato.plan.handoff", "outcome": "maybe", "approval": "00000000-0000-0000-0000-000000000005"}
      """
    Then the companion answers 400
    When the launched session's skill reports a "committed" handoff without its launch header
    Then the companion answers 400
    And no handoff report arrives within 1 second

  Scenario: A server that does not take handoff reports gets none
    Given the server does not announce "handoff_reports"
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session's skill reports a "committed" handoff to the companion
    Then the companion answers 202
    And the companion logs "handoff report not forwarded: the server did not announce handoff_reports"
    And no handoff report arrives within 1 second
