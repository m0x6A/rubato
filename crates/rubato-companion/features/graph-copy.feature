Feature: A companion keeps a copy of the team's context graph
  The server announces its context graph sources on the websocket; the companion fetches each
  one that differs from its copy over the server URL, with its pairing token, and applies it to
  graph.sqlite in its state directory only after the snapshot hashed to what was announced
  (ADR 0086, plan 329 design §2). The copy flows in; nothing leaves the machine for it.

  Scenario: The Hello names context_graph
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the companion daemon is running and connected
    Then the server receives a hello announcing "context_graph"

  Scenario: An announced source is fetched with the pairing token and applied to the copy
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the test server also announces "context_graph"
    And the test server holds the context graph source "repo/acme/pantry" at head "abc1234" with the file "src/stock.rs"
    And the companion daemon is running and connected
    When the server announces its context graph sources
    Then the companion's copy holds "repo/acme/pantry" at head "abc1234" with the file "src/stock.rs"
    And every snapshot was asked for with the pairing token

  Scenario: A snapshot that is not the one announced is not applied
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the test server also announces "context_graph"
    And the test server holds the context graph source "repo/acme/pantry" at head "abc1234" with the file "src/stock.rs"
    And the test server serves another body for "repo/acme/pantry" than it announces
    And the companion daemon is running and connected
    When the server announces its context graph sources
    Then the snapshot of "repo/acme/pantry" was asked for
    And the companion's copy holds no source

  Scenario: A session started by hand reads the copy through the stdio bridge
    Given a test server that approves pairing for user "m0x6A"
    And a paired companion for user "m0x6A"
    And the test server also announces "context_graph"
    And the test server holds the context graph source "repo/acme/pantry" at head "abc1234" with the file "src/stock.rs"
    And the companion daemon is running and connected
    When the server announces its context graph sources
    Then the companion's copy holds "repo/acme/pantry" at head "abc1234" with the file "src/stock.rs"
    And a process speaking MCP over stdio to the companion's bridge lists 5 read-only tools
    And asked through the bridge, graph_status names "repo/acme/pantry" at head "abc1234"
    And in a session Rubato launched the bridge lists no tools
