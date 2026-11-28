Feature: Map drafts
  The team edits the map as one shared draft and proposes it as a pull request on the map file.

  @drafts.add-part-and-seam
  Scenario: A part and a seam added to the draft show on the map for everyone
    When a person adds a part and draws a seam to it in edit mode
    Then both are on the map, dashed and marked with who drafted them

  @drafts.invalid-change-explained
  Scenario: A change the map would refuse is not added, and says why
    When a person adds a part with an id the map already has
    Then the part is not added
    And they are told the id already exists

  @drafts.propose-as-pull-request
  Scenario: Proposing the draft opens one pull request on the map repository
    Given a draft with changes
    When a person proposes it and confirms the diff
    Then one branch holding the new map file is pushed
    And a pull request is opened for it

  @drafts.withdraw
  Scenario: Withdrawing a proposal keeps every change in the draft
    Given a proposed draft
    When a person withdraws the proposal
    Then the branch is gone
    And the draft is editable again with every change kept

  @drafts.ghost-into-draft
  Scenario: A ghost taken into the draft becomes a part, and its call a seam
    Given a ghost service seen in the observed traffic
    When a person takes it into the draft
    Then it is a draft part that names its service
    And its observed call is a draft seam

  @drafts.one-next-step
  Scenario: The map draft's view offers one next step
    Given a draft with changes in it
    When a person opens the map draft
    Then they see the changes, and Propose map change as the one next step
    And adding a part or a seam opens its form when asked for, by its button, a double-click on the canvas or something taken from the observed system
    And copying the file and discarding the draft are behind More, while Undo discard stays in sight
