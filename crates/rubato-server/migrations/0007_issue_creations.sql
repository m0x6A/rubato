-- Issue #71, PR 4 (ADR 0025). Additive: one table.

-- One row per "New issue here…" request, by the id the form made. At most one issue per id:
-- `opening` until GitHub answers or the listing shows an issue with the `Rubato-Request:` line,
-- then `created`; `failed` when refused or unseen for ten minutes. Owner: new_issue.
CREATE TABLE issue_creations (
    request_id  TEXT PRIMARY KEY,
    repo        TEXT NOT NULL,
    kind        TEXT NOT NULL,          -- node | seam: where it was started, and is pinned
    target      TEXT NOT NULL,
    title       TEXT NOT NULL,
    created_by  TEXT NOT NULL,          -- GitHub login of the person
    created_at  TEXT NOT NULL,
    state       TEXT NOT NULL,          -- opening | created | failed
    number      INTEGER,
    url         TEXT,
    error       TEXT,
    updated_at  TEXT NOT NULL
);
