-- Issue #71, PR 3 (ADR 0026). Additive: two nullable columns and one table.

-- An untrusted `Map:` line and the suggested place of a listed open issue, as JSON
-- (`MapRef`, `PinSuggestion`). Replaced with the rest of the row per listing. Owner: github_poll.
ALTER TABLE open_issues ADD COLUMN map_line TEXT;
ALTER TABLE open_issues ADD COLUMN suggestion TEXT;

-- Every (issue, target) a `Map:` line has been applied for, pinned or found already pinned. A
-- line is applied once: after a person unpins the issue, the same line does not pin it again.
-- Owner: pins.
CREATE TABLE map_line_pins (
    repo        TEXT NOT NULL,
    number      INTEGER NOT NULL,
    kind        TEXT NOT NULL,          -- node | seam
    target      TEXT NOT NULL,
    outcome     TEXT NOT NULL,          -- pinned | already_pinned
    applied_at  TEXT NOT NULL,
    PRIMARY KEY (repo, number, kind, target)
);
