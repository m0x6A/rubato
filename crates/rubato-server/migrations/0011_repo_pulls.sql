-- Issue #82, PR 1 (ADR 0034). Additive: two tables.

-- Every open pull request from a same-repo branch of the repos the map names, replaced per
-- listing. Checks are kept across listings while the head does not move. Owner: github_poll.
CREATE TABLE repo_pulls (
    repo        TEXT NOT NULL,
    number      INTEGER NOT NULL,
    branch      TEXT NOT NULL,
    base        TEXT NOT NULL,
    title       TEXT NOT NULL,
    author      TEXT NOT NULL,
    draft       INTEGER NOT NULL,
    head_sha    TEXT NOT NULL,
    checks      TEXT,                   -- ChecksStatus as snake_case; NULL until read
    checks_sha  TEXT,                   -- the head the checks were read at
    updated_at  TEXT NOT NULL,
    url         TEXT NOT NULL,
    PRIMARY KEY (repo, number)
);

-- How fresh each repo's pull request listing is. Owner: github_poll.
CREATE TABLE repo_pulls_status (
    repo        TEXT PRIMARY KEY,
    listed_at   TEXT,                   -- last good listing; NULL until the first
    truncated   INTEGER NOT NULL DEFAULT 0,
    error       TEXT                    -- why the latest listing failed; NULL when it succeeded
);
