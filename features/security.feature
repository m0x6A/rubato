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

  @security.tls-off-loopback
  Scenario: A server off loopback does not start on plain http unless it is told to
    Given a public URL that is not https and a bind address that is not loopback
    When the server starts without RUBATO_ALLOW_HTTP=1
    Then it does not start, and says the public URL is not https
    And with RUBATO_ALLOW_HTTP=1 it starts, as on a LAN

  @security.secrets-required
  Scenario: A server on the internet does not start without its secrets
    Given an https public URL off loopback
    When the session secret, the token key, the GitHub App or the operators are missing
    Then the server does not start, and names each one that is missing

  @security.hsts-and-host-cookie
  Scenario: Over https the browser is told to stay on https and the session cookie cannot be set from elsewhere
    Given a server whose public URL is https
    Then every response carries Strict-Transport-Security
    And the session cookie is named with the __Host- prefix, Secure, with no Domain

  @security.anonymous-rate-limit
  Scenario: An address that floods login or pairing is told to wait
    When one address sends more requests to login or pairing than its burst
    Then the answer is 429 with Retry-After
    And another address is still answered

  @security.write-rate-limit
  Scenario: A session that writes too fast is told to wait, and can still read
    When one session sends more writes than its burst
    Then the answer is 429 with Retry-After
    And the same session still reads, and another session still writes

  @security.forwarded-only-from-proxy
  Scenario: A client cannot choose the address it is counted as
    Given a server behind a trusted proxy
    When a client sends its own X-Forwarded-For
    Then it is counted by the address the proxy saw

  @security.pairing-codes-per-address
  Scenario: One address cannot hold all the pending pairing codes
    When one address has asked for its share of pairing codes
    Then its next one is refused
    And a companion at another address still pairs

  @security.request-timeout
  Scenario: A request that runs too long is ended
    When a request runs past 30 seconds
    Then it is answered 503
    And a websocket is not ended this way
