Feature: A plan's room
  The team talks about a plan in its room. An agent joins only when its owner calls it in.

  @room.team-talks
  Scenario: Two people talk in a plan's room
    Given an issue with a plan
    When two people write in the plan's room
    Then each sees both messages, in order

  @room.ask-owner-to-call-agent
  Scenario: A teammate asks an owner to call their agent in
    Given a person's session on the plan's branch
    When a teammate asks that person to call their agent in
    Then the owner sees the request with Accept and Decline

  @room.owner-calls-agent-in
  Scenario: The owner accepts and their agent joins the room
    Given a request to call a person's agent into the room
    When the owner accepts it
    Then their agent is in the room
