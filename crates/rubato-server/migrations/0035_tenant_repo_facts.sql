-- Issue #345 (ADR 0087; design §3.1, §8). The Repo-fact tables gain `tenant_id`: a repository
-- belongs to at most one tenant, and every fact about it is that tenant's. Each table is rebuilt
-- with `tenant_id TEXT NOT NULL` and no default, in every key and unique index that names a
-- repo, so an insert that forgets it fails. Every existing row goes to tenant `default` (the
-- seed's). Rows keyed by an id of their own (a draft, an approval, a room message) keep that
-- key and carry the tenant beside it. No table has a foreign key.

CREATE TABLE open_issues_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    labels TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    map_line TEXT,
    suggestion TEXT,
    author TEXT NOT NULL DEFAULT '',
    author_association TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (tenant_id, repo, number)
);
INSERT INTO open_issues_new (tenant_id, repo, number, title, url, labels, updated_at, map_line, suggestion, author, author_association)
    SELECT 'default', repo, number, title, url, labels, updated_at, map_line, suggestion, author, author_association FROM open_issues;
DROP TABLE open_issues;
ALTER TABLE open_issues_new RENAME TO open_issues;

CREATE TABLE backlog_repos_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    listed_at TEXT,
    truncated INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    PRIMARY KEY (tenant_id, repo)
);
INSERT INTO backlog_repos_new (tenant_id, repo, listed_at, truncated, error)
    SELECT 'default', repo, listed_at, truncated, error FROM backlog_repos;
DROP TABLE backlog_repos;
ALTER TABLE backlog_repos_new RENAME TO backlog_repos;

CREATE TABLE uncovered_repos_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    seen_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, repo)
);
INSERT INTO uncovered_repos_new (tenant_id, repo, seen_at)
    SELECT 'default', repo, seen_at FROM uncovered_repos;
DROP TABLE uncovered_repos;
ALTER TABLE uncovered_repos_new RENAME TO uncovered_repos;

CREATE TABLE pull_requests_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    pr TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, repo, number)
);
INSERT INTO pull_requests_new (tenant_id, repo, number, pr, fetched_at)
    SELECT 'default', repo, number, pr, fetched_at FROM pull_requests;
DROP TABLE pull_requests;
ALTER TABLE pull_requests_new RENAME TO pull_requests;

CREATE TABLE decisions_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    comment_id INTEGER NOT NULL,
    record TEXT NOT NULL,
    PRIMARY KEY (tenant_id, repo, number, comment_id)
);
INSERT INTO decisions_new (tenant_id, repo, number, comment_id, record)
    SELECT 'default', repo, number, comment_id, record FROM decisions;
DROP TABLE decisions;
ALTER TABLE decisions_new RENAME TO decisions;

CREATE TABLE issue_meta_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    title TEXT,
    github_state TEXT,
    url TEXT,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, repo, number)
);
INSERT INTO issue_meta_new (tenant_id, repo, number, title, github_state, url, fetched_at)
    SELECT 'default', repo, number, title, github_state, url, fetched_at FROM issue_meta;
DROP TABLE issue_meta;
ALTER TABLE issue_meta_new RENAME TO issue_meta;

CREATE TABLE plan_signals_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    comment_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    record TEXT NOT NULL,
    target_sha TEXT,
    PRIMARY KEY (tenant_id, repo, number, comment_id)
);
INSERT INTO plan_signals_new (tenant_id, repo, number, comment_id, kind, record, target_sha)
    SELECT 'default', repo, number, comment_id, kind, record, target_sha FROM plan_signals;
DROP TABLE plan_signals;
ALTER TABLE plan_signals_new RENAME TO plan_signals;

CREATE TABLE repo_pulls_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    base TEXT NOT NULL,
    title TEXT NOT NULL,
    author TEXT NOT NULL,
    draft INTEGER NOT NULL,
    head_sha TEXT NOT NULL,
    checks TEXT,
    checks_sha TEXT,
    updated_at TEXT NOT NULL,
    url TEXT NOT NULL,
    PRIMARY KEY (tenant_id, repo, number)
);
INSERT INTO repo_pulls_new (tenant_id, repo, number, branch, base, title, author, draft, head_sha, checks, checks_sha, updated_at, url)
    SELECT 'default', repo, number, branch, base, title, author, draft, head_sha, checks, checks_sha, updated_at, url FROM repo_pulls;
DROP TABLE repo_pulls;
ALTER TABLE repo_pulls_new RENAME TO repo_pulls;

CREATE TABLE repo_pulls_status_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    listed_at TEXT,
    truncated INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    PRIMARY KEY (tenant_id, repo)
);
INSERT INTO repo_pulls_status_new (tenant_id, repo, listed_at, truncated, error)
    SELECT 'default', repo, listed_at, truncated, error FROM repo_pulls_status;
DROP TABLE repo_pulls_status;
ALTER TABLE repo_pulls_status_new RENAME TO repo_pulls_status;

CREATE TABLE plans_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    plan TEXT,
    parse_error TEXT,
    fetched_at TEXT NOT NULL,
    approval_sha TEXT,
    head_sha TEXT,
    PRIMARY KEY (tenant_id, repo, number)
);
INSERT INTO plans_new (tenant_id, repo, number, branch, path, plan, parse_error, fetched_at, approval_sha, head_sha)
    SELECT 'default', repo, number, branch, path, plan, parse_error, fetched_at, approval_sha, head_sha FROM plans;
DROP TABLE plans;
ALTER TABLE plans_new RENAME TO plans;

CREATE TABLE plan_approvals_new (
    tenant_id TEXT NOT NULL,
    login TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    sha TEXT NOT NULL,
    branch TEXT NOT NULL,
    state TEXT NOT NULL,
    comment_id INTEGER,
    comment_url TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, login, repo, number, sha)
);
INSERT INTO plan_approvals_new (tenant_id, login, repo, number, sha, branch, state, comment_id, comment_url, created_at)
    SELECT 'default', login, repo, number, sha, branch, state, comment_id, comment_url, created_at FROM plan_approvals;
DROP TABLE plan_approvals;
ALTER TABLE plan_approvals_new RENAME TO plan_approvals;

CREATE TABLE rubato_approvals_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    draft_id TEXT NOT NULL,
    rev INTEGER NOT NULL,
    plan_sha TEXT NOT NULL,
    bundle_hash TEXT NOT NULL,
    approved_by TEXT NOT NULL,
    approved_at TEXT NOT NULL,
    self INTEGER NOT NULL,
    summary TEXT NOT NULL,
    handoff_outcome TEXT,
    handoff_at TEXT,
    created_at TEXT NOT NULL,
    handoff_commit TEXT,
    handoff_adrs_left TEXT,
    UNIQUE (tenant_id, repo, number, branch, plan_sha, bundle_hash)
);
INSERT INTO rubato_approvals_new (tenant_id, id, repo, number, branch, draft_id, rev, plan_sha, bundle_hash, approved_by, approved_at, self, summary, handoff_outcome, handoff_at, created_at, handoff_commit, handoff_adrs_left)
    SELECT 'default', id, repo, number, branch, draft_id, rev, plan_sha, bundle_hash, approved_by, approved_at, self, summary, handoff_outcome, handoff_at, created_at, handoff_commit, handoff_adrs_left FROM rubato_approvals;
DROP TABLE rubato_approvals;
ALTER TABLE rubato_approvals_new RENAME TO rubato_approvals;

CREATE TABLE plan_links_new (
    tenant_id TEXT NOT NULL,
    approval_id TEXT PRIMARY KEY NOT NULL,
    commit_sha TEXT NOT NULL,
    state TEXT NOT NULL,
    comment_id INTEGER,
    url TEXT,
    created_at TEXT NOT NULL,
    posted_at TEXT
);
INSERT INTO plan_links_new (tenant_id, approval_id, commit_sha, state, comment_id, url, created_at, posted_at)
    SELECT 'default', approval_id, commit_sha, state, comment_id, url, created_at, posted_at FROM plan_links;
DROP TABLE plan_links;
ALTER TABLE plan_links_new RENAME TO plan_links;

CREATE TABLE plan_drafts_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    owner TEXT NOT NULL,
    created_at TEXT NOT NULL,
    imported_from TEXT,
    original_author TEXT,
    conflict_user TEXT,
    conflict_at TEXT,
    UNIQUE (tenant_id, repo, number, branch)
);
INSERT INTO plan_drafts_new (tenant_id, id, repo, number, branch, owner, created_at, imported_from, original_author, conflict_user, conflict_at)
    SELECT 'default', id, repo, number, branch, owner, created_at, imported_from, original_author, conflict_user, conflict_at FROM plan_drafts;
DROP TABLE plan_drafts;
ALTER TABLE plan_drafts_new RENAME TO plan_drafts;

CREATE TABLE plan_draft_revisions_new (
    tenant_id TEXT NOT NULL,
    draft_id TEXT NOT NULL,
    rev INTEGER NOT NULL,
    launch_id TEXT,
    session_id TEXT,
    files TEXT NOT NULL,
    missing TEXT NOT NULL,
    plan_sha TEXT NOT NULL,
    bundle_hash TEXT NOT NULL,
    plan TEXT,
    parse_error TEXT,
    at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, draft_id, rev)
);
INSERT INTO plan_draft_revisions_new (tenant_id, draft_id, rev, launch_id, session_id, files, missing, plan_sha, bundle_hash, plan, parse_error, at)
    SELECT 'default', draft_id, rev, launch_id, session_id, files, missing, plan_sha, bundle_hash, plan, parse_error, at FROM plan_draft_revisions;
DROP TABLE plan_draft_revisions;
ALTER TABLE plan_draft_revisions_new RENAME TO plan_draft_revisions;

CREATE TABLE pr_merges_new (
    tenant_id TEXT NOT NULL,
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
    PRIMARY KEY (tenant_id, repo, number, head_sha)
);
INSERT INTO pr_merges_new (tenant_id, repo, number, head_sha, login, method, state, merge_sha, listings_open, opened_at, updated_at)
    SELECT 'default', repo, number, head_sha, login, method, state, merge_sha, listings_open, opened_at, updated_at FROM pr_merges;
DROP TABLE pr_merges;
ALTER TABLE pr_merges_new RENAME TO pr_merges;

CREATE TABLE room_messages_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    author_kind TEXT NOT NULL,
    author TEXT NOT NULL,
    text TEXT NOT NULL,
    at TEXT NOT NULL,
    notice_seam TEXT,
    notice_repo TEXT,
    notice_number INTEGER,
    notice_branch TEXT,
    addressed TEXT,
    turn_id TEXT,
    spoken INTEGER NOT NULL DEFAULT 0
);
INSERT INTO room_messages_new (tenant_id, id, repo, number, branch, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch, addressed, turn_id, spoken)
    SELECT 'default', id, repo, number, branch, author_kind, author, text, at, notice_seam, notice_repo, notice_number, notice_branch, addressed, turn_id, spoken FROM room_messages;
DROP TABLE room_messages;
ALTER TABLE room_messages_new RENAME TO room_messages;
CREATE INDEX room_messages_by_room ON room_messages (tenant_id, repo, number, branch, at);
CREATE UNIQUE INDEX room_messages_one_notice ON room_messages (tenant_id, repo, number, branch, notice_repo, notice_number, notice_seam) WHERE notice_seam IS NOT NULL;
CREATE UNIQUE INDEX room_messages_one_reply ON room_messages (turn_id, author) WHERE turn_id IS NOT NULL AND author_kind = 'agent';

CREATE TABLE room_agents_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    launch_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    called_by TEXT NOT NULL,
    joined_at TEXT NOT NULL,
    left_at TEXT,
    left_reason TEXT,
    last_turn_at TEXT,
    fork_session_id TEXT
);
INSERT INTO room_agents_new (tenant_id, id, repo, number, branch, launch_id, owner, called_by, joined_at, left_at, left_reason, last_turn_at, fork_session_id)
    SELECT 'default', id, repo, number, branch, launch_id, owner, called_by, joined_at, left_at, left_reason, last_turn_at, fork_session_id FROM room_agents;
DROP TABLE room_agents;
ALTER TABLE room_agents_new RENAME TO room_agents;
CREATE UNIQUE INDEX room_agents_one_per_owner ON room_agents (tenant_id, repo, number, branch, owner COLLATE NOCASE) WHERE left_at IS NULL;

CREATE TABLE room_requests_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    owner TEXT NOT NULL,
    launch_id TEXT NOT NULL,
    requested_by TEXT NOT NULL,
    at TEXT NOT NULL,
    state TEXT NOT NULL,
    answered_at TEXT
);
INSERT INTO room_requests_new (tenant_id, id, repo, number, branch, owner, launch_id, requested_by, at, state, answered_at)
    SELECT 'default', id, repo, number, branch, owner, launch_id, requested_by, at, state, answered_at FROM room_requests;
DROP TABLE room_requests;
ALTER TABLE room_requests_new RENAME TO room_requests;
CREATE INDEX room_requests_by_room ON room_requests (tenant_id, repo, number, branch, state);

CREATE TABLE spend_sessions_new (
    tenant_id TEXT NOT NULL,
    companion_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    usd REAL NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, companion_id, session_id)
);
INSERT INTO spend_sessions_new (tenant_id, companion_id, session_id, repo, number, usd, at)
    SELECT 'default', companion_id, session_id, repo, number, usd, at FROM spend_sessions;
DROP TABLE spend_sessions;
ALTER TABLE spend_sessions_new RENAME TO spend_sessions;
CREATE INDEX spend_sessions_issue ON spend_sessions (tenant_id, repo, number);

CREATE TABLE spend_points_new (
    tenant_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    bucket TEXT NOT NULL,
    usd REAL NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, repo, number, bucket)
);
INSERT INTO spend_points_new (tenant_id, repo, number, bucket, usd, at)
    SELECT 'default', repo, number, bucket, usd, at FROM spend_points;
DROP TABLE spend_points;
ALTER TABLE spend_points_new RENAME TO spend_points;

CREATE TABLE session_log_entries_new (
    tenant_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    prompt_id TEXT NOT NULL,
    role TEXT NOT NULL,
    launch_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    truncated INTEGER NOT NULL,
    message_id TEXT,
    mixed INTEGER NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, session_id, prompt_id, role)
);
INSERT INTO session_log_entries_new (tenant_id, session_id, prompt_id, role, launch_id, repo, number, kind, text, truncated, message_id, mixed, at)
    SELECT 'default', session_id, prompt_id, role, launch_id, repo, number, kind, text, truncated, message_id, mixed, at FROM session_log_entries;
DROP TABLE session_log_entries;
ALTER TABLE session_log_entries_new RENAME TO session_log_entries;
CREATE INDEX session_log_by_launch ON session_log_entries (launch_id, at);

