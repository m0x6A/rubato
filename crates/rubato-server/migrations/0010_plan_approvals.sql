-- Issue #80, PR 3 (ADR 0030). Additive: one table. Owner: approve.

-- An approval Rubato posted, or is posting, as a person: one row per (person, issue, plan
-- version), so a double click, a retry or a second tab posts once.
CREATE TABLE plan_approvals (
    login        TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    sha          TEXT NOT NULL,
    branch       TEXT NOT NULL,
    state        TEXT NOT NULL,      -- opening or posted
    comment_id   INTEGER,
    comment_url  TEXT,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (login, repo, number, sha)
);
