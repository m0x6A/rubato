-- Issue #48, PR 2 (ADR 0016). Additive: one table.

-- Where each part sits on the shared map; last write wins per part. Presentation only, never
-- an input to progress. Owner: layout.
CREATE TABLE map_layout (
    node_id     TEXT PRIMARY KEY,
    x           REAL NOT NULL,
    y           REAL NOT NULL,
    updated_by  TEXT NOT NULL,          -- GitHub login of who moved it last
    updated_at  TEXT NOT NULL
);
