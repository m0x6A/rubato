Feature: Telemetry
  Rubato is treated like production: every part reports what it does, in the names the
  protocol keeps.

  @telemetry.session-attributes
  Scenario: A launched session exports telemetry tagged with its repo, issue, branch and part
    When a person's companion launches a session on a pinned issue
    Then the session runs with telemetry turned on
    And its resource attributes name the repo, the issue, the branch and the part

  @telemetry.both-binaries-export
  Scenario: The server and the companion export their metrics and spans
    Given a server and a companion with an OTLP endpoint
    When sessions run through the flow
    Then each exports the metrics and spans on-call relies on

  @telemetry.no-per-session-metrics
  Scenario: No metric from the companion carries a series per session
    When the companion exports metrics
    Then none counts sessions by state, the server's alone
    And none carries the session id
