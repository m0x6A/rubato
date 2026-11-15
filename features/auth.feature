Feature: Logging in
  A person logs in with GitHub. Rubato lets in only accounts that can reach the team's GitHub App
  installation (ADR 0022), and never shows the team's data to anyone else.

  @auth.login-page
  Scenario: Without a session, a visitor sees the login page and the API refuses
    Given a visitor with no Rubato session
    When they open Rubato
    Then they see the login page
    And every API call answers 401

  @auth.member-logs-in
  Scenario: A team member logs in and sees what the installation lists
    Given an account that can reach the App installation
    When they log in with GitHub
    Then they see the map and the issues of the installation's repositories

  @auth.no-access-refused
  Scenario: An account with no access to the installation is refused
    Given an account that cannot reach the App installation
    When they log in with GitHub
    Then they are told they have no access
    And no Rubato session is made for them

  @auth.lookup-fault
  Scenario: A failed installation lookup is reported as a fault, not as no access
    Given GitHub fails while Rubato checks the installation
    When a person logs in
    Then they are told something went wrong and to try again
    And they are not told they have no access

  @auth.session-ends-mid-work
  Scenario: When the session ends mid-work, the next write leads back to login and writes nothing
    Given a logged-in team member with the map open
    And their session has ended
    When they make a change
    Then they are taken back to the login page
    And nothing was written

  @auth.log-out
  Scenario: A person logs out from the app
    Given a logged-in team member
    When they choose to log out
    Then they see the login page
    And their session no longer works
