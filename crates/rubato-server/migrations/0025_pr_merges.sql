-- Issue #82, ADR 0033: a merge as the person, attempted at most once per (repo, pull request,
-- head). Owned by slice `pulls`. `opening` until GitHub answers or the poll finds out; `merged`
-- rows are kept 7 days. `listings_open` counts complete listings that still showed the pull
-- request open at this head after a merge whose outcome was unknown (plan 82 Decision 12).
CREATE TABLE pr_merges (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    login TEXT NOT NULL,
    method TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('opening', 'merged')),
    merge_sha TEXT,
    listings_open INTEGER NOT NULL DEFAULT 0,
    opened_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo, number, head_sha)
);
