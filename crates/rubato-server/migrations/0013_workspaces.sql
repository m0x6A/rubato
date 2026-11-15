-- Issue #89, PR 3 (ADRs 0044, 0047). Several workspaces on one server, each one map.
--
-- Not additive: five tables are rebuilt with `workspace_id` in their key. Every existing row
-- becomes workspace `default`, which a workspaces file must keep (ADR 0044). The rebuilt tables
-- have no column default, so a store call that forgets the workspace fails instead of writing to
-- `default`. Columns added with ALTER need a default (SQLite requires one for NOT NULL).
-- Rollback: restore the database copy taken before the deploy, with the previous sha (design D5).
-- No table has a foreign key, so dropping and renaming is safe. A text PRIMARY KEY is also NOT NULL:
-- SQLite lets a non-integer primary key hold NULL otherwise.

-- Owner: layout. One position per part per workspace.
CREATE TABLE map_layout_new (
    workspace_id TEXT NOT NULL,
    node_id      TEXT NOT NULL,
    x            REAL NOT NULL,
    y            REAL NOT NULL,
    updated_by   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (workspace_id, node_id)
);
INSERT INTO map_layout_new (workspace_id, node_id, x, y, updated_by, updated_at)
    SELECT 'default', node_id, x, y, updated_by, updated_at FROM map_layout;
DROP TABLE map_layout;
ALTER TABLE map_layout_new RENAME TO map_layout;

-- Owner: map. The last good map of each workspace.
CREATE TABLE map_cache_new (
    workspace_id TEXT NOT NULL PRIMARY KEY,
    version      TEXT NOT NULL,
    yaml         TEXT NOT NULL,
    loaded_at    TEXT NOT NULL
);
INSERT INTO map_cache_new (workspace_id, version, yaml, loaded_at)
    SELECT 'default', version, yaml, loaded_at FROM map_cache;
DROP TABLE map_cache;
ALTER TABLE map_cache_new RENAME TO map_cache;

-- Owner: map_draft. At most one proposal per workspace.
CREATE TABLE map_proposal_new (
    workspace_id TEXT NOT NULL PRIMARY KEY,
    branch       TEXT NOT NULL,
    base_sha     TEXT NOT NULL,
    file_hash    TEXT NOT NULL,
    last_seq     INTEGER NOT NULL,
    state        TEXT NOT NULL,
    state_since  TEXT NOT NULL,
    pr_number    INTEGER,
    pr_url       TEXT,
    proposed_by  TEXT NOT NULL
);
INSERT INTO map_proposal_new (workspace_id, branch, base_sha, file_hash, last_seq, state, state_since, pr_number, pr_url, proposed_by)
    SELECT 'default', branch, base_sha, file_hash, last_seq, state, state_since, pr_number, pr_url, proposed_by FROM map_proposal;
DROP TABLE map_proposal;
ALTER TABLE map_proposal_new RENAME TO map_proposal;

-- Owner: pins. A pin is a statement about one workspace's map.
CREATE TABLE pins_new (
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    target       TEXT NOT NULL,
    pinned_by    TEXT NOT NULL,
    pinned_at    TEXT NOT NULL,
    PRIMARY KEY (workspace_id, repo, number, kind, target)
);
INSERT INTO pins_new (workspace_id, repo, number, kind, target, pinned_by, pinned_at)
    SELECT 'default', repo, number, kind, target, pinned_by, pinned_at FROM pins;
DROP TABLE pins;
ALTER TABLE pins_new RENAME TO pins;

-- Owner: pins. A `Map:` line applied once per workspace.
CREATE TABLE map_line_pins_new (
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    target       TEXT NOT NULL,
    outcome      TEXT NOT NULL,
    applied_at   TEXT NOT NULL,
    PRIMARY KEY (workspace_id, repo, number, kind, target)
);
INSERT INTO map_line_pins_new (workspace_id, repo, number, kind, target, outcome, applied_at)
    SELECT 'default', repo, number, kind, target, outcome, applied_at FROM map_line_pins;
DROP TABLE map_line_pins;
ALTER TABLE map_line_pins_new RENAME TO map_line_pins;

-- Owner: map_draft. `seq` stays global and monotonic; reads filter by workspace.
ALTER TABLE map_draft_ops ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';
CREATE INDEX map_draft_ops_workspace ON map_draft_ops (workspace_id, seq);

-- Owners: new_issue, launch. Where an issue was started from.
ALTER TABLE issue_creations ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE launches ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';

-- Owner: github_poll. A suggestion is scored against one map, so it is kept per workspace.
-- `open_issues.suggestion` is no longer written; `open_issues.map_line` stays, a GitHub fact.
CREATE TABLE issue_suggestions (
    workspace_id TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    suggestion   TEXT NOT NULL,
    PRIMARY KEY (workspace_id, repo, number)
);
INSERT INTO issue_suggestions (workspace_id, repo, number, suggestion)
    SELECT 'default', repo, number, suggestion FROM open_issues WHERE suggestion IS NOT NULL;
UPDATE open_issues SET suggestion = NULL;
