-- Issue #122, PR 5 (ADR 0053). Plan drafts that live in Rubato until approval, mirrored by the
-- companion from a launched session's worktree at its settle points. Owner: slice plan_drafts.
-- Additive. One draft per (repo, number, branch); a revision is stored only when the bundle hash
-- changes. `conflict_user` and `conflict_at` record the last bundle refused because another
-- person owns the draft (finding DraftOwnerConflict). Every visible state (live, parked, approved,
-- on branch) is derived, never stored.
CREATE TABLE plan_drafts (
    id              TEXT PRIMARY KEY NOT NULL,
    repo            TEXT NOT NULL,
    number          INTEGER NOT NULL,
    branch          TEXT NOT NULL,
    owner           TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    imported_from   TEXT,
    original_author TEXT,
    conflict_user   TEXT,
    conflict_at     TEXT,
    UNIQUE (repo, number, branch)
);

CREATE TABLE plan_draft_revisions (
    draft_id    TEXT NOT NULL,
    rev         INTEGER NOT NULL,
    launch_id   TEXT,
    session_id  TEXT,
    files       TEXT NOT NULL,
    missing     TEXT NOT NULL,
    plan_sha    TEXT NOT NULL,
    bundle_hash TEXT NOT NULL,
    plan        TEXT,
    parse_error TEXT,
    at          TEXT NOT NULL,
    PRIMARY KEY (draft_id, rev)
);
