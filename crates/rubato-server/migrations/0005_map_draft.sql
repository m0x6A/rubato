-- Issue #48, PR 6 (ADR 0018). Additive: two tables.

-- The shared draft of the map: operations in order. Owner: map_draft.
CREATE TABLE map_draft_ops (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    op_id         TEXT NOT NULL UNIQUE,   -- made by the browser; a retried request adds nothing
    op            TEXT NOT NULL,          -- MapOp as JSON
    author        TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    discarded_at  TEXT                    -- set by Discard, cleared by Undo
);

-- The one proposal there can be (id is always 1). Owner: map_draft.
CREATE TABLE map_proposal (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    branch       TEXT NOT NULL,
    base_sha     TEXT NOT NULL,           -- head commit of the map branch at proposal
    file_hash    TEXT NOT NULL,           -- SHA-256 of the map file the proposal rewrites
    last_seq     INTEGER NOT NULL,        -- last operation in the proposal
    state        TEXT NOT NULL,           -- opening, proposed or merged
    state_since  TEXT NOT NULL,
    pr_number    INTEGER,
    pr_url       TEXT,
    proposed_by  TEXT NOT NULL
);
