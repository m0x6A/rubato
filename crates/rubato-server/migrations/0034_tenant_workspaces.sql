-- Issue #345 (ADR 0087, 0089; design §3.1, §8). The Workspace-class tables gain `tenant_id`
-- in every key: a workspace id is unique only inside its tenant. Each table is rebuilt with
-- `tenant_id TEXT NOT NULL` and no default, so an insert that forgets it fails instead of
-- landing in the first team's tenant. Every existing row goes to tenant `default` (the seed's).
-- No table has a foreign key. The Repo-fact and Launch tables follow in their own migrations.

-- Owner: workspace_settings.
CREATE TABLE workspace_settings_new (
    tenant_id              TEXT NOT NULL,
    workspace_id           TEXT NOT NULL,
    position               INTEGER NOT NULL,
    revision               INTEGER NOT NULL DEFAULT 1,
    name                   TEXT,
    map_repo               TEXT NOT NULL,
    map_path               TEXT NOT NULL,
    map_ref                TEXT NOT NULL DEFAULT 'main',
    traffic_source         TEXT,
    traffic_fields         TEXT,
    traffic_secrets_sealed TEXT,
    default_spend_cap_usd  REAL,
    default_attempt_limit  INTEGER,
    updated_by             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id)
);
INSERT INTO workspace_settings_new (tenant_id, workspace_id, position, revision, name, map_repo, map_path, map_ref, traffic_source, traffic_fields, traffic_secrets_sealed, default_spend_cap_usd, default_attempt_limit, updated_by, updated_at)
    SELECT 'default', workspace_id, position, revision, name, map_repo, map_path, map_ref, traffic_source, traffic_fields, traffic_secrets_sealed, default_spend_cap_usd, default_attempt_limit, updated_by, updated_at FROM workspace_settings;
DROP TABLE workspace_settings;
ALTER TABLE workspace_settings_new RENAME TO workspace_settings;
-- One map is one workspace's inside a tenant; a repository belongs to one tenant (ADR 0087).
CREATE UNIQUE INDEX workspace_settings_one_map
    ON workspace_settings (tenant_id, map_repo COLLATE NOCASE, map_path, map_ref);

-- Owner: layout.
CREATE TABLE map_layout_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    node_id      TEXT NOT NULL,
    x            REAL NOT NULL,
    y            REAL NOT NULL,
    updated_by   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id, node_id)
);
INSERT INTO map_layout_new (tenant_id, workspace_id, node_id, x, y, updated_by, updated_at)
    SELECT 'default', workspace_id, node_id, x, y, updated_by, updated_at FROM map_layout;
DROP TABLE map_layout;
ALTER TABLE map_layout_new RENAME TO map_layout;

CREATE TABLE map_layout_grouped_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('part', 'box')),
    id           TEXT NOT NULL,
    x            REAL NOT NULL,
    y            REAL NOT NULL,
    updated_by   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id, kind, id)
);
INSERT INTO map_layout_grouped_new (tenant_id, workspace_id, kind, id, x, y, updated_by, updated_at)
    SELECT 'default', workspace_id, kind, id, x, y, updated_by, updated_at FROM map_layout_grouped;
DROP TABLE map_layout_grouped;
ALTER TABLE map_layout_grouped_new RENAME TO map_layout_grouped;

-- Owner: map.
CREATE TABLE map_cache_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    version      TEXT NOT NULL,
    yaml         TEXT NOT NULL,
    loaded_at    TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id)
);
INSERT INTO map_cache_new (tenant_id, workspace_id, version, yaml, loaded_at)
    SELECT 'default', workspace_id, version, yaml, loaded_at FROM map_cache;
DROP TABLE map_cache;
ALTER TABLE map_cache_new RENAME TO map_cache;

-- Owner: map_draft.
CREATE TABLE map_proposal_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    branch       TEXT NOT NULL,
    base_sha     TEXT NOT NULL,
    file_hash    TEXT NOT NULL,
    last_seq     INTEGER NOT NULL,
    state        TEXT NOT NULL,
    state_since  TEXT NOT NULL,
    pr_number    INTEGER,
    pr_url       TEXT,
    proposed_by  TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id)
);
INSERT INTO map_proposal_new (tenant_id, workspace_id, branch, base_sha, file_hash, last_seq, state, state_since, pr_number, pr_url, proposed_by)
    SELECT 'default', workspace_id, branch, base_sha, file_hash, last_seq, state, state_since, pr_number, pr_url, proposed_by FROM map_proposal;
DROP TABLE map_proposal;
ALTER TABLE map_proposal_new RENAME TO map_proposal;

CREATE TABLE map_draft_ops_new (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    tenant_id     TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    op_id         TEXT NOT NULL UNIQUE,   -- made by the browser; a retried request adds nothing
    op            TEXT NOT NULL,          -- MapOp as JSON
    author        TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    discarded_at  TEXT                    -- set by Discard, cleared by Undo
);
INSERT INTO map_draft_ops_new (seq, tenant_id, workspace_id, op_id, op, author, created_at, discarded_at)
    SELECT seq, 'default', workspace_id, op_id, op, author, created_at, discarded_at FROM map_draft_ops;
DROP TABLE map_draft_ops;
ALTER TABLE map_draft_ops_new RENAME TO map_draft_ops;
CREATE INDEX map_draft_ops_workspace ON map_draft_ops (tenant_id, workspace_id, seq);

-- Owner: pins.
CREATE TABLE pins_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    target       TEXT NOT NULL,
    pinned_by    TEXT NOT NULL,
    pinned_at    TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id, repo, number, kind, target)
);
INSERT INTO pins_new (tenant_id, workspace_id, repo, number, kind, target, pinned_by, pinned_at)
    SELECT 'default', workspace_id, repo, number, kind, target, pinned_by, pinned_at FROM pins;
DROP TABLE pins;
ALTER TABLE pins_new RENAME TO pins;

CREATE TABLE map_line_pins_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    target       TEXT NOT NULL,
    outcome      TEXT NOT NULL,
    applied_at   TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id, repo, number, kind, target)
);
INSERT INTO map_line_pins_new (tenant_id, workspace_id, repo, number, kind, target, outcome, applied_at)
    SELECT 'default', workspace_id, repo, number, kind, target, outcome, applied_at FROM map_line_pins;
DROP TABLE map_line_pins;
ALTER TABLE map_line_pins_new RENAME TO map_line_pins;

-- Owner: new_issue.
CREATE TABLE issue_creations_new (
    request_id   TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    kind         TEXT NOT NULL,
    target       TEXT NOT NULL,
    title        TEXT NOT NULL,
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    state        TEXT NOT NULL,
    number       INTEGER,
    url          TEXT,
    error        TEXT,
    updated_at   TEXT NOT NULL
);
INSERT INTO issue_creations_new (request_id, tenant_id, workspace_id, repo, kind, target, title, created_by, created_at, state, number, url, error, updated_at)
    SELECT request_id, 'default', workspace_id, repo, kind, target, title, created_by, created_at, state, number, url, error, updated_at FROM issue_creations;
DROP TABLE issue_creations;
ALTER TABLE issue_creations_new RENAME TO issue_creations;

-- Owner: github_poll.
CREATE TABLE issue_suggestions_new (
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    suggestion   TEXT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id, repo, number)
);
INSERT INTO issue_suggestions_new (tenant_id, workspace_id, repo, number, suggestion)
    SELECT 'default', workspace_id, repo, number, suggestion FROM issue_suggestions;
DROP TABLE issue_suggestions;
ALTER TABLE issue_suggestions_new RENAME TO issue_suggestions;

-- Owner: traffic.
CREATE TABLE traffic_dismissals_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    target_key   TEXT NOT NULL,
    author       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE (tenant_id, workspace_id, target_key)
);
INSERT INTO traffic_dismissals_new (id, tenant_id, workspace_id, target_key, author, created_at)
    SELECT id, 'default', workspace_id, target_key, author, created_at FROM traffic_dismissals;
DROP TABLE traffic_dismissals;
ALTER TABLE traffic_dismissals_new RENAME TO traffic_dismissals;
