# Design: a pairing approval never moves a companion to another person (#160)

Plan: [../160.md](../160.md). Full track: an auth change on the `companion-server` and
`web-server` seams (a new refusal on `POST /api/pair/approve`, and a code that ends `denied`).

## The hole, as the code stands at e36d1f3

1. `GET /api/overview` lists every pairing with its `companion_id` to every logged-in member
   (`crates/rubato-server/src/overview/handler.rs:30`). That is by design: the lanes are shared.
2. `POST /api/pair/start` is anonymous and stores the `companion_id` the caller sends
   (`crates/rubato-server/src/pairing/handlers.rs:19-33`).
3. `approve_pairing_code` upserts `pairings` with `ON CONFLICT(companion_id) DO UPDATE SET user =
   excluded.user, token_hash = excluded.token_hash, ... revoked_at = NULL`
   (`crates/rubato-server/src/pairing/store.rs:87`). Nothing compares the existing row's `user`
   with the approver.

So member A starts a pairing with B's `companion_id`, approves their own code, and the row is now
A's with A's token hash. B's companion gets 401 on reconnect, B cannot revoke it
(`store.rs:158` filters on `user`), and A owns B's lane, queue and spend history. B's machine is
still safe: the companion refuses a launch for any user but its paired one
(`crates/rubato-companion/src/launch/launcher.rs:51`).

ADR 0011 already says the token is "bound to (user, companion id)". The fix makes the store keep
that binding: a `companion_id` belongs to the first user who paired it, for good.

## The change

One statement decides, so two approvals cannot interleave between a read and a write:

```sql
INSERT INTO pairings (companion_id, user, token_hash, hostname, created_at)
VALUES (?, ?, ?, ?, ?)
ON CONFLICT(companion_id) DO UPDATE SET
  token_hash = excluded.token_hash, hostname = excluded.hostname, revoked_at = NULL
WHERE pairings.user = excluded.user
```

`user` is no longer in the `SET` list; the `WHERE` makes the upsert a no-op when the row belongs to
someone else, and `rows_affected() == 0` says so. In that case the same transaction marks the code
`denied` (the status already exists in `migrations/0001_init.sql:24`) and commits, so:

- the approver gets **409** with a sentence ("This companion is paired to another person. ...");
- the companion polling that code gets `Denied` at once instead of waiting for expiry;
- the code cannot be approved a second time;
- B's row, token hash and `revoked_at` are untouched;
- `rubato.web.rejected{rubato.route=/api/pair/approve, rubato.reason=companion_taken}` is counted
  and the info line `web write rejected` names the approver (`AppState::reject`,
  `crates/rubato-server/src/kernel/state.rs:164`).

`approve_pairing_code` returns an enum instead of `Option<Uuid>`:

```rust
pub enum Approval { Approved(Uuid), NotFound, Taken(Uuid) }
```

The handler maps `NotFound` to 404 as today and `Taken` to 409.

## What stays

- Re-pairing one's own companion (same `companion_id`, same user) replaces the token and clears
  `revoked_at`, as today. `pair_and_save` keeps the id across pairings
  (`crates/rubato-companion/src/pairing/pair.rs:29`), so this is the normal path.
- The overview still lists every pairing's `companion_id`. Hiding it would break the shared lanes
  and buys nothing once the id cannot be claimed.
- The companion needs no change: `Denied` already ends `pair` with "pairing was denied"
  (`pair.rs:95`). The web needs no change: it shows the error text of a refused approve
  (`web/src/features/companions/CompanionsPanel.tsx:23`).

## Revoked rows of another person (question Q1 in the plan)

With the `WHERE` above, a row stays its first user's even after revocation. That is stricter than
the issue's "a live pairing": a revoked row still carries B's spend history and identity, and
handing it to A would show B's history under A's name. The cost: a machine passed from B to A
must drop its `companion.json` (a new `companion_id`) before A can pair it, and A sees "pairing
was denied" until then. The plan asks the maintainer to choose; the code implements the stricter
rule.

Server-minted ids (the issue's other suggestion) would close the hole too, but change
`PairStartRequest` and `PairPollResponse` in `rubato-protocol`, the companion and the schema, for
the same protection. Listed as an alternative, not chosen.

## Tests

`crates/rubato-server/tests/integration.rs`:

- `a_teammate_cannot_take_over_another_persons_companion`: B pairs X; A starts with X and
  approves: 409; A's code polls `Denied`; B's pairing is unchanged (user B, not revoked); B's
  token still opens `/ws/companion`.
- In `crates/rubato-server/src/app.rs`, `refusals_are_counted_by_route_and_reason` gains the
  `companion_taken` reason on `/api/pair/approve` (it holds the in-process metric reader).
- `a_revoked_companion_stays_with_its_owner`: B pairs X and revokes it; A approves X: 409; B
  approves X again: 200, `revoked_at` cleared.
- `pairing_your_own_companion_again_replaces_the_token`: B pairs X twice; the first token is
  refused (401), the second connects.

Each fails on today's code: the first two get 200, and the third already passes (it guards the
path the fix must not break).
