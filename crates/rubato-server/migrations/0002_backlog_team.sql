-- Issue #48, PR 1 (ADR 0019, 0022). Additive: three tables and one nullable column.

-- Every open issue of the repos the map names, replaced per listing. Owner: github_poll.
CREATE TABLE open_issues (
    repo        TEXT NOT NULL,
    number      INTEGER NOT NULL,
    title       TEXT NOT NULL,
    url         TEXT NOT NULL,
    labels      TEXT NOT NULL,          -- JSON array of label names
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (repo, number)
);

-- How fresh each repo's listing is: the trust check for the backlog. Owner: github_poll.
CREATE TABLE backlog_repos (
    repo        TEXT PRIMARY KEY,
    listed_at   TEXT,                   -- last good listing; NULL until the first
    truncated   INTEGER NOT NULL DEFAULT 0,
    error       TEXT                    -- why the latest listing failed; NULL when it succeeded
);

-- Repos the map or a tracked issue names that the App installation does not cover. Owner: github_poll.
CREATE TABLE uncovered_repos (
    repo        TEXT PRIMARY KEY,
    seen_at     TEXT NOT NULL
);

-- When this session's login was last seen to have access to the App's installation.
-- NULL means "log in again" once the App is configured. Owner: auth.
ALTER TABLE web_sessions ADD COLUMN team_checked_at TEXT;
