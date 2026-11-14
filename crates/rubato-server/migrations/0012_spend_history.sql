-- Issue #103, PR 1 (ADR 0037). Additive: two tables.

-- The latest spend per session, with the issue it was first recorded under. The key starts with
-- the companion id of the pairing the snapshot arrived on, not a claim in the snapshot.
-- Owner: spend_history.
CREATE TABLE spend_sessions (
    companion_id TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    usd          REAL NOT NULL,
    at           TEXT NOT NULL,         -- when the snapshot carrying it was received
    PRIMARY KEY (companion_id, session_id)
);
CREATE INDEX spend_sessions_issue ON spend_sessions (repo, number);

-- Per issue and 15-minute bucket, the issue's total over every session it has had.
-- Owner: spend_history.
CREATE TABLE spend_points (
    repo   TEXT NOT NULL,
    number INTEGER NOT NULL,
    bucket TEXT NOT NULL,               -- received time floored to 15 minutes, RFC 3339
    usd    REAL NOT NULL,
    at     TEXT NOT NULL,               -- the receipt time of the last write into the bucket
    PRIMARY KEY (repo, number, bucket)
);
