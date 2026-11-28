Feature: Logging in
  A person logs in with GitHub. Rubato lets in only people who were invited to a tenant, and
  operators (ADR 0088), and never shows a tenant's data to anyone else.

  @auth.login-page
  Scenario: Without a session, a visitor sees the login page and the API refuses
    Given a visitor with no Rubato session
    When they open Rubato
    Then they see the login page
    And every API call answers 401

  @auth.member-logs-in
  Scenario: An invited person logs in and sees what their tenant's installation lists
    Given a person an admin invited to a tenant
    When they log in with GitHub
    Then they see the map and the issues of the tenant's repositories

  @auth.no-access-refused
  Scenario: An account nobody invited is refused
    Given an account that no tenant's admin has invited
    When they log in with GitHub
    Then they are told they have not been invited
    And no Rubato session is made for them

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
