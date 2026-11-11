-- Issue #48, PR 5 (ADR 0020). Additive: one table.

-- Issues a developer intends to run on one of their own companions, in order. Stored intent,
-- not progress: nothing here is read by the progress derivation. Owner: queue.
CREATE TABLE companion_queue (
    companion_id      TEXT NOT NULL,
    repo              TEXT NOT NULL,
    number            INTEGER NOT NULL,
    position          INTEGER NOT NULL,
    queued_by         TEXT NOT NULL,
    queued_at         TEXT NOT NULL,
    last_start_error  TEXT,             -- why the latest start did not start; NULL until one fails
    PRIMARY KEY (companion_id, repo, number)
);
