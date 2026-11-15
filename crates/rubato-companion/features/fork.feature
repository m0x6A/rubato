Feature: Room turns, typed while planning and run in a read-only fork while building
  A person addressed the owner's agent in the plan's room (ADR 0057). While the plan is a draft
  the turn is typed into the working session at idle, like a chat message, and its reply comes
  back through the session log. Once the plan is on the branch the turn runs in a read-only fork
  of the session (claude -p --resume <session> --fork-session --permission-mode plan) with no
  Rubato hooks; the answer, the fork's session and its cost go back as a RoomReply. The fork's
  cost counts against the launch's spend cap.

  Background:
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And a temporary repository "acme/pantry" with a bare origin

  Scenario: The Hello names room turns
    Given the companion daemon is running and connected
    Then the server receives a hello announcing "room_turns"

  Scenario: While planning a room turn is typed at idle like a chat message
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the launched session posts a "Stop" hook
    And the server sends the launched session a room turn while planning
    Then the server receives the outcome "delivered" for that message
    And the stand-in claude was typed the room turn
    And there was 0 fork runs

  Scenario: While building the first turn forks the session without Rubato hooks
    Given a fork turn costs 0.25 dollars
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the server sends the launched session a room turn while building
    Then the server receives the outcome "delivered" for that message
    And the server receives a room reply "b keeps it in one place" costing 0.25 from fork session "fork-of-<session>"
    And fork run 1 resumed the launched session with "-p --fork-session --permission-mode plan"
    And the fork ran with no Rubato hooks and the turn on its stdin
    And nothing was typed into the stand-in claude

  Scenario: A later turn resumes the fork, not the session
    Always forking would start from the working session again and lose the room's thread: with
    --fork-session on every turn this scenario fails.
    Given the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the server sends the launched session a room turn while building
    Then the server receives a room reply "b keeps it in one place" costing 0.05 from fork session "fork-of-<session>"
    When the server sends the launched session a room turn resuming that fork
    Then the server receives a room reply "b keeps it in one place" costing 0.05 from fork session "fork-of-<session>"
    And fork run 2 resumed the fork without "--fork-session"

  Scenario: A fork that passes the spend cap is a cap hit, and the next turn does not run
    Given a fork turn costs 30 dollars
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the server sends the launched session a room turn while building
    Then the server receives a room reply "b keeps it in one place" costing 30 from fork session "fork-of-<session>"
    And the server receives a spend cap hit for the launched issue
    When the server sends the launched session a room turn resuming that fork
    Then the server receives the outcome "failed" for that message
    And the server receives a room reply with the error "spend cap reached"
    And there was 1 fork run

  Scenario: A server that does not take room replies gets no turn run
    Given the server does not announce "room_replies"
    And the companion daemon is running and connected
    And the server has launched issue 7 of "acme/pantry" with slug "restock" for user "m0x6A"
    When the server sends the launched session a room turn while building
    Then the server receives the outcome "failed" for that message
    And there was 0 fork runs
