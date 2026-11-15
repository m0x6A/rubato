-- Issue #140 (ADR 0060). Additive. Owner: traffic.
-- A discrepancy between the map and the observed system that a person dismissed for a
-- workspace: a ghost part, an edge no seam declares, or a quiet seam. `target_key` is the
-- canonical JSON of the `DiscrepancyKey`; undo deletes the row.
CREATE TABLE traffic_dismissals (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    target_key   TEXT NOT NULL,
    author       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE (workspace_id, target_key)
);
