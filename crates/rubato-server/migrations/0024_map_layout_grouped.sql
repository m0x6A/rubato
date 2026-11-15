-- Issue #144 (ADR 0068). Additive: one table, owned by `layout`.
--
-- The By repo grouping's shared arrangement, next to the flat map's `map_layout` and never
-- mixed with it. kind 'part': a part's top-left relative to its repo's box (on the canvas for a
-- part with no repo), id is the part id. kind 'box': a repo box's top-left on the canvas, id is
-- the repo as `owner/name`. Last write wins per row. Presentation only, never an input to
-- progress, pins or plans.
CREATE TABLE map_layout_grouped (
    workspace_id TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('part', 'box')),
    id           TEXT NOT NULL,
    x            REAL NOT NULL,
    y            REAL NOT NULL,
    updated_by   TEXT NOT NULL,          -- GitHub login of who moved it last
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (workspace_id, kind, id)
);
