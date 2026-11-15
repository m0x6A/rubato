Feature: Security of the page
  The page runs under a strict Content-Security-Policy and the server's security headers
  (ADR 0066).

  @security.content-security-policy
  Scenario: The app works under its Content-Security-Policy with no violation
    When a person opens Rubato and uses the map
    Then the page is served with a policy that allows only its own origin by default
    And nothing on the page violates the policy

  @security.same-origin-writes
  Scenario: A write from another origin is refused
    Given a page on another origin
    When it sends a write to Rubato's API in a logged-in person's browser
    Then Rubato refuses it and writes nothing
