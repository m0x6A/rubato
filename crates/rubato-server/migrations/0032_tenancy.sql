-- Issue #345 (ADR 0087, 0088; design §3). Additive, owned by `tenants`.
--
-- A tenant owns GitHub App installations, workspaces and invited members. An installation
-- belongs to at most one tenant, so a repository does too: `tenant_repos.repo` is the key. The
-- `tenant_id` column on the workspace, repo-fact and launch tables comes with the pull request
-- that scopes each table's queries (design §13), so nothing here touches an existing table.
CREATE TABLE tenants (
    id          TEXT PRIMARY KEY,            -- [a-z0-9-]{1,32}, as workspace ids
    name        TEXT NOT NULL,
    created_by  TEXT NOT NULL,               -- a login, or 'seed'
    created_at  TEXT NOT NULL
);

CREATE TABLE tenant_installations (
    installation_id  INTEGER PRIMARY KEY,    -- GitHub's id; one tenant at most
    tenant_id        TEXT NOT NULL,
    account_login    TEXT,                   -- NULL until GitHub has said (the seed knows only the id)
    account_type     TEXT,                   -- 'User' or 'Organization'; NULL as above
    attached_by      TEXT NOT NULL,
    attached_at      TEXT NOT NULL,
    suspended_at     TEXT,                   -- GitHub suspended it: not read
    detached_at      TEXT                    -- detached or deleted: rows kept, hidden, until purged
);
CREATE INDEX tenant_installations_tenant ON tenant_installations (tenant_id);

-- A member is a GitHub user id: a login can be renamed and reused (ADR 0088). `login` is for
-- display, refreshed at each login. `joined_at` is NULL until the invited person logs in.
CREATE TABLE tenant_members (
    tenant_id   TEXT NOT NULL,
    github_id   INTEGER NOT NULL,
    login       TEXT NOT NULL,
    role        TEXT NOT NULL,               -- 'admin' or 'member'
    invited_by  TEXT NOT NULL,
    invited_at  TEXT NOT NULL,
    joined_at   TEXT,
    PRIMARY KEY (tenant_id, github_id)
);
CREATE INDEX tenant_members_person ON tenant_members (github_id);

CREATE TABLE tenant_repos (
    repo             TEXT NOT NULL PRIMARY KEY COLLATE NOCASE,
    tenant_id        TEXT NOT NULL,
    installation_id  INTEGER NOT NULL,
    seen_at          TEXT NOT NULL,          -- the last listing that named it
    detached_at      TEXT                    -- its installation was detached: hidden, not read
);
CREATE INDEX tenant_repos_tenant ON tenant_repos (tenant_id);
CREATE INDEX tenant_repos_installation ON tenant_repos (installation_id);

-- Who a login is, by id, for invites and the identity guard (Decision 15).
CREATE INDEX users_github_id ON users (github_id);
