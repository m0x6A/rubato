Feature: Plans
  The agent plans from the map; the plan comes back into the shared picture, is read by the
  team, and is approved in Rubato.

  @plans.plan-comes-back
  Scenario: A session's plan comes back and passes the plan check
    Given sessions started on pinned issues
    When each session pushes its plan to the issue branch
    Then the server reads each plan
    And the plan check passes over every plan that came back

  @plans.paints-the-map
  Scenario: Selecting an issue with a plan paints its parts and flows
    Given an issue whose branch holds a plan with affected parts and flows
    When a person selects the issue
    Then the plan's parts are painted and every other part steps back
    And the plan's flows move along their seams
    And the panel says what the plan says

  @plans.found-by-branch
  Scenario: A plan of an issue nobody pinned is found by its branch and shows where it fits
    Given an issue nobody pinned whose issue branch holds a plan
    When a person opens Plans
    Then the plan is listed, found by its branch, with whether it fits the map
    And the parts it names show it as planned there

  @plans.unfit-cannot-be-accepted
  Scenario: A plan that does not fit the map cannot be accepted
    Given a plan that names a part the map does not have
    When a person opens Plans
    Then the plan is marked as not fitting
    And Accept plan is disabled

  @plans.draft-revisions
  Scenario: The team reads every revision of a draft plan
    Given a session drafting a plan in Rubato
    When the session settles a new revision
    Then everyone can read the draft and each earlier revision

  @plans.approve-in-rubato
  Scenario: A teammate approves the newest revision, and the plan reaches the branch once
    Given a settled draft plan
    When a teammate approves its newest revision in Rubato
    Then the session commits the plan once, with the approval record
    And the issue gets one link comment from the approver

  @plans.import
  Scenario: A plan written outside Rubato becomes the importer's draft
    Given a plan pushed to an issue branch from a session started by hand
    When a person presses Import on the issue
    Then a plan that fits becomes their draft in Rubato
