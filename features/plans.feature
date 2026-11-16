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

  @plans.done-view
  Scenario: A finished plan is kept and not shown by default
    Given a plan whose issue is closed on GitHub
    When a person opens Plans
    Then the plan is not listed
    And it is still read from the issue under Done, with nothing to accept or approve

  @plans.board
  Scenario: Plans are read on a board of their own
    Given plans of issues that are open on GitHub, proposed and open
    When a person opens Plans with the panel widened
    Then Proposed and Open are columns side by side, and a plan that is only a draft is among the proposed
    And a plan opens in the Plans tab, with where it fits and its draft, while the map paints it
    And the issue is one press away

  @plans.reads-as-markdown
  Scenario: A plan's text reads as a document
    Given a plan draft with headings, tables and a task list
    When a person opens its plan file
    Then the text is rendered as Markdown, with the raw text one press away
    And HTML, scripts and images in the plan put nothing on the page but text and links
