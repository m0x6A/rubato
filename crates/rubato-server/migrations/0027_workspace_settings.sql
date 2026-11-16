-- Issue #189 (ADR 0075). Additive, owned by `workspace_settings`: one row per workspace, the
-- settings that describe the application it shows. Seeded once from RUBATO_WORKSPACES_FILE (or
-- RUBATO_MAP_*) and the RUBATO_TRAFFIC_* / RUBATO_DEFAULT_* variables; the database owns them
-- after that. `map_file` is never stored: it is a path on the server's disk, read by id at start.
CREATE TABLE workspace_settings (
    workspace_id           TEXT PRIMARY KEY,
    -- Switcher order: file order at seed, new workspaces last.
    position               INTEGER NOT NULL,
    -- 1 at insert, +1 per change: the compare-and-set token of an edit.
    revision               INTEGER NOT NULL DEFAULT 1,
    name                   TEXT,
    map_repo               TEXT NOT NULL,
    map_path               TEXT NOT NULL,
    map_ref                TEXT NOT NULL DEFAULT 'main',
    -- The traffic adapter's name (`prometheus`); NULL: no source.
    traffic_source         TEXT,
    -- JSON object of the adapter's text fields; only the adapter reads them.
    traffic_fields         TEXT,
    -- JSON object of the adapter's secrets, sealed with RUBATO_TOKEN_KEY; never returned.
    traffic_secrets_sealed TEXT,
    -- NULL: the built-in defaults (25 USD, 3 attempts).
    default_spend_cap_usd  REAL,
    default_attempt_limit  INTEGER,
    updated_by             TEXT NOT NULL,
    updated_at             TEXT NOT NULL
);
-- Two workspaces never name the same map (ADR 0044), compared without case on the repo.
CREATE UNIQUE INDEX workspace_settings_one_map
    ON workspace_settings (map_repo COLLATE NOCASE, map_path, map_ref);
