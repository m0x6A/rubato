Feature: Navigation
  Every view has an address: the URL's fragment names what the panel shows, and the part or seam
  selected on the map beside it. A person can reload, go back and send a link.

  @navigation.reload-keeps-view
  Scenario: Reloading keeps the view
    Given a person with an issue open and a part selected on the map
    When they reload the page
    Then the same issue is open and the same part is selected

  @navigation.back-returns
  Scenario: Back returns to the previous view
    Given a person who went from the issue list to an issue and then to the plans
    When they go back in the browser
    Then the issue is open again
    When they go back once more
    Then they see the issue list

  @navigation.link-opens-issue
  Scenario: A link opens an issue
    Given the address of an issue's view
    When a person opens that address in a new page
    Then the issue is open, under Issues

  @navigation.unknown-link
  Scenario: A link that names nothing says so
    Given an address whose fragment names no view, or an issue Rubato does not track
    When a person opens it
    Then the page says what it could not find
    And offers the way to the list

  @navigation.login-returns
  Scenario: Logging in returns to the view the link named
    Given a visitor with no Rubato session who opened the address of the plans
    When they log in with GitHub
    Then they see the plans, not the issue list
