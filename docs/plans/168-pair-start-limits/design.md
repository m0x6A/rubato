# Design: bound anonymous pairing starts, sweep expired codes, approve a code once (#168)

Plan: [../168.md](../168.md). Full track: a change to the `companion-server` contract (two new
refusals on the anonymous `POST /api/pair/start`) and to what `POST /api/pair/approve` answers a
second approver on `web-server`.

## The hole, as the code stands at e36d1f3

- `POST /api/pair/start` has no auth and no rate limit (`crates/rubato-server/src/app.rs:86`, the
  route list). Each call inserts a `pairing_codes` row; `hostname` and `companion_version` are
  bounded only by axum's 2 MB body limit (`crates/rubato-server/src/pairing/handlers.rs:19-33`).
- Nothing deletes from `pairing_codes`. A pending row is marked `expired` only when it is polled
  (`crates/rubato-server/src/pairing/store.rs:54`).
- `approve_pairing_code` reads the code, checks `status = 'pending'` in Rust, then updates it with
  no guard (`store.rs:67-89`). Two approvals of one code that both pass the read both write: two
  `pairings` upserts and two tokens parked on the code, the last one winning.

Anyone who can reach the server can grow its SQLite file without bound.

## The change

### 1. Field lengths (422)

`hostname` and `companion_version` are refused above `PAIR_FIELD_MAX_CHARS = 255` characters
(counted as `chars()`, not bytes), with 422 and a sentence naming the field. A DNS name is at most
253 characters, so no real companion is refused. The constant lives in
`rubato_protocol::pairing` next to the route, like `QUEUE_MAX_PER_COMPANION` in `queue`; it is a
`const`, so `schema/protocol/*.json` does not change.

### 2. A global cap on pending codes (429)

At most `PAIRING_CODES_MAX_PENDING = 50` codes may be pending and unexpired at once. The check and
the insert are one statement, so concurrent starts cannot overshoot:

```sql
INSERT INTO pairing_codes (...)
SELECT ?, ?, ?, ?, ?, 'pending', ?, ?
WHERE (SELECT COUNT(*) FROM pairing_codes WHERE status = 'pending' AND expires_at > ?) < ?
```

Zero rows inserted means the cap is reached: 429 with a sentence ("Too many pairings are waiting
for approval; try again in a few minutes."). Pending codes expire after `pairing_ttl_secs` (900 s),
so the cap frees itself.

Global, not per IP: the server usually sits behind a reverse proxy, where the peer address is the
proxy's, and `X-Forwarded-For` is whatever the caller writes. A flood can therefore hold off a real
pairing for up to one TTL; it can no longer grow the database. That trade is question Q1 in the
plan.

Timestamps are compared as stored strings. `ts()` writes `to_rfc3339()` in UTC
(`crates/rubato-server/src/kernel/db.rs:27`), whose fixed-width date and time sort correctly, and
the token sweep already compares them this way (`crates/rubato-server/src/auth/tokens.rs:298`).

### 3. Sweep expired codes in the poll loop

`pairing::sweep_codes` deletes every `pairing_codes` row whose `expires_at` is more than
`PAIRING_CODE_KEEP_SECS = 3600` in the past, whatever its status. It runs once per poll tick in
`tasks::run`, next to the tick and outside it, so a missing map or a failing GitHub call does not
stop it. It logs `pairing codes swept` at info with `gone` when it deleted any.

The hour of grace keeps a late poll answered `Expired` rather than `Denied`. A code approved but
never polled loses its parked token with its row; the companion gave up at the same expiry.

Bound: at most 50 pending codes per 15 minutes, kept up to an hour after expiry plus one tick:
about 250 rows of at most ~600 bytes.

### 4. Approve a code once

The update that claims the code carries the guard and its result is read:

```sql
UPDATE pairing_codes SET status = 'approved', user = ?, token = ?
WHERE device_code = ? AND status = 'pending' AND expires_at > ?
```

It runs first in the transaction; zero rows means another approval (or expiry) won, so the
transaction is dropped and the caller gets 404, as for any code that is not pending. The pairing
upsert runs only after a successful claim. SQLite serializes writers, and a deferred transaction
whose first statement is a write reads the latest commit, so the second approver's update sees
`approved` and matches nothing.

## What stays

- The companion: `pair` already fails on any non-2xx start (`error_for_status`,
  `crates/rubato-companion/src/pairing/pair.rs:67`) and prints the error.
- Normal pairing, polling and approval answer as before (every integration test that uses
  `Harness::pair`).

## Interaction with #160

#160 (PR on `issue/160-pairing-takeover`) changes the pairing upsert in the same function. This
branch only adds the guard to the claim and moves no other line, so the two merge with a small,
mechanical conflict in `approve_pairing_code`, whichever lands second.

## Tests

- `pair_start_refuses_long_fields` (integration): 256-character hostname, 256-character version:
  422 each; 255 characters: 200. Fails today (200).
- `pair_start_caps_pending_codes` (integration, cap set to 3 in config): three starts 200, the
  fourth 429; after one is approved, a start is 200 again. Fails today.
- `sweep_deletes_codes_long_expired` (store unit test): a code expired two hours ago is deleted, one
  expired a minute ago and a pending one are kept, and the count returned is 1. Fails today (no
  sweep).
- `claim_refuses_a_code_already_claimed` (store unit test): the claim step, called directly on a
  code another approval already claimed (the state a racing approver meets after its read),
  returns false and leaves the code's user and token as the first approval set them. Today the
  step has no guard; the test is written against the extracted `claim_code` and fails until the
  guard is in.
- `concurrent_approvals_of_one_code_pair_once` (integration): ten users approve one code at
  once; exactly one gets 200 and the rest 404, and `pairings` holds one row for the companion.
  A race test can pass by luck; the unit test above is the one that fails on purpose.
- `refusals_are_counted_by_route_and_reason` (`app.rs`): one `field_too_long` and one
  `pending_cap` on `/api/pair/start`.
