-- Issue #345 (ADR 0087; design §3.1, §8). The Launch tables gain `tenant_id`: a launch, what is
-- queued for a companion, a message to a session, an agent's report, a lead's request and a
-- cap hit each belong to the tenant of the issue's repository. Each table is rebuilt with
-- `tenant_id TEXT NOT NULL` and no default; every existing row goes to tenant `default`.
-- `launches.workspace_id` loses its default too: a launch names its workspace.

CREATE TABLE launches_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY,
    companion_id TEXT NOT NULL,
    user TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    command TEXT NOT NULL,
    result TEXT,
    created_at TEXT NOT NULL,
    resulted_at TEXT,
    workspace_id TEXT NOT NULL,
    role TEXT,
    lead_launch_id TEXT
);
INSERT INTO launches_new (tenant_id, id, companion_id, user, repo, number, command, result, created_at, resulted_at, workspace_id, role, lead_launch_id)
    SELECT 'default', id, companion_id, user, repo, number, command, result, created_at, resulted_at, workspace_id, role, lead_launch_id FROM launches;
DROP TABLE launches;
ALTER TABLE launches_new RENAME TO launches;
CREATE INDEX launches_by_lead ON launches (lead_launch_id);
CREATE INDEX launches_by_user ON launches (user, created_at);

CREATE TABLE companion_queue_new (
    tenant_id TEXT NOT NULL,
    companion_id TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    position INTEGER NOT NULL,
    queued_by TEXT NOT NULL,
    queued_at TEXT NOT NULL,
    last_start_error TEXT,
    PRIMARY KEY (tenant_id, companion_id, repo, number)
);
INSERT INTO companion_queue_new (tenant_id, companion_id, repo, number, position, queued_by, queued_at, last_start_error)
    SELECT 'default', companion_id, repo, number, position, queued_by, queued_at, last_start_error FROM companion_queue;
DROP TABLE companion_queue;
ALTER TABLE companion_queue_new RENAME TO companion_queue;

CREATE TABLE session_messages_new (
    tenant_id TEXT NOT NULL,
    id TEXT PRIMARY KEY NOT NULL,
    launch_id TEXT NOT NULL,
    session_id TEXT,
    companion_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    priority INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    pushed_at TEXT,
    outcome_at TEXT,
    detail TEXT,
    via_launch TEXT
);
INSERT INTO session_messages_new (tenant_id, id, launch_id, session_id, companion_id, owner, repo, number, kind, body, author, priority, state, created_at, pushed_at, outcome_at, detail, via_launch)
    SELECT 'default', id, launch_id, session_id, companion_id, owner, repo, number, kind, body, author, priority, state, created_at, pushed_at, outcome_at, detail, via_launch FROM session_messages;
DROP TABLE session_messages;
ALTER TABLE session_messages_new RENAME TO session_messages;
CREATE INDEX session_messages_by_launch ON session_messages (launch_id, state);
CREATE INDEX session_messages_by_companion ON session_messages (companion_id, state);

CREATE TABLE agent_reports_new (
    tenant_id TEXT NOT NULL,
    launch_id TEXT NOT NULL,
    companion_id TEXT NOT NULL,
    doing TEXT NOT NULL,
    parts TEXT NOT NULL,
    reported_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, launch_id)
);
INSERT INTO agent_reports_new (tenant_id, launch_id, companion_id, doing, parts, reported_at)
    SELECT 'default', launch_id, companion_id, doing, parts, reported_at FROM agent_reports;
DROP TABLE agent_reports;
ALTER TABLE agent_reports_new RENAME TO agent_reports;
CREATE INDEX agent_reports_companion ON agent_reports (companion_id);

CREATE TABLE fleet_requests_new (
    tenant_id TEXT NOT NULL,
    companion_id TEXT NOT NULL,
    lead_launch_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    action_hash TEXT NOT NULL,
    action TEXT NOT NULL,
    owner TEXT NOT NULL,
    target_launch_id TEXT,
    state TEXT NOT NULL,
    outcome TEXT,
    reason TEXT,
    result TEXT,
    claimed_at TEXT NOT NULL,
    done_at TEXT,
    PRIMARY KEY (tenant_id, companion_id, lead_launch_id, request_id)
);
INSERT INTO fleet_requests_new (tenant_id, companion_id, lead_launch_id, request_id, action_hash, action, owner, target_launch_id, state, outcome, reason, result, claimed_at, done_at)
    SELECT 'default', companion_id, lead_launch_id, request_id, action_hash, action, owner, target_launch_id, state, outcome, reason, result, claimed_at, done_at FROM fleet_requests;
DROP TABLE fleet_requests;
ALTER TABLE fleet_requests_new RENAME TO fleet_requests;
CREATE INDEX fleet_requests_by_age ON fleet_requests (claimed_at);

CREATE TABLE cap_hits_new (
    tenant_id TEXT NOT NULL,
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    payload TEXT NOT NULL,
    at TEXT NOT NULL
);
INSERT INTO cap_hits_new (tenant_id, id, repo, number, payload, at)
    SELECT 'default', id, repo, number, payload, at FROM cap_hits;
DROP TABLE cap_hits;
ALTER TABLE cap_hits_new RENAME TO cap_hits;

