# 0011. Authenticate the companion with an opaque pairing token from the device-code flow

- **Status:** accepted
- **Date:** 2026-11-08
- **Decided by:** Mattias Jonsson (drafted by agent, Claude Code v0 planning session)
- **Seam:** companion-server
- **Issue:** none

## Context

After pairing, every message from a companion must be attributable to one GitHub login, and
a lost laptop must be revocable without touching GitHub.

## Decision

The device-code flow ends with the server issuing a random 256-bit pairing token bound to
(user, companion id). The server stores only its hash; the companion stores it in a 0600
file under its config dir. The token is sent as a bearer on the websocket upgrade. The web
view lists pairings and revokes them; a revoked token drops the connection and counts
`rubato.events.dropped{reason=auth}`.

## Alternatives considered

- **Hand the companion the user's GitHub OAuth token**: wider scope than needed; expires on GitHub's terms.
- **Short-lived JWT with refresh**: more moving parts for one team.
- **mTLS**: certificate handling on every laptop.

## Consequences

Easier: revocation, attribution. Harder: a secret file per machine; the pairing Gherkin
scenario covers loss and re-pair.

## Supersedes

none
