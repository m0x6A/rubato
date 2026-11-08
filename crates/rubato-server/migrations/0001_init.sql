-- Rubato server schema. Progress is derived at read time; no table stores it (AGENTS.md).

CREATE TABLE users (
    login TEXT PRIMARY KEY,
    github_id INTEGER,
    avatar_url TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE web_sessions (
    id TEXT PRIMARY KEY,
    login TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Device-code pairing (ADR 0011). The token is handed to the companion once and only its hash is kept.
CREATE TABLE pairing_codes (
    device_code TEXT PRIMARY KEY,
    user_code TEXT NOT NULL UNIQUE,
    companion_id TEXT NOT NULL,
    hostname TEXT NOT NULL,
    companion_version TEXT NOT NULL,
    status TEXT NOT NULL,            -- pending | approved | denied | expired | consumed
    user TEXT,
    token TEXT,                      -- cleared once polled
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE pairings (
    companion_id TEXT PRIMARY KEY,
    user TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    hostname TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked_at TEXT,
    last_seen TEXT,
    server_seq INTEGER NOT NULL DEFAULT 0,
    last_companion_seq INTEGER NOT NULL DEFAULT 0
);

-- The one piece of state GitHub does not hold (ADR 0008).
CREATE TABLE pins (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    kind TEXT NOT NULL,              -- node | seam
    target TEXT NOT NULL,
    pinned_by TEXT NOT NULL,
    pinned_at TEXT NOT NULL,
    PRIMARY KEY (repo, number, kind, target)
);

CREATE TABLE map_cache (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version TEXT NOT NULL,
    yaml TEXT NOT NULL,
    loaded_at TEXT NOT NULL
);

-- Every inbound envelope, for traces and replay. Signals, not state.
CREATE TABLE companion_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    companion_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    received_at TEXT NOT NULL,
    UNIQUE (companion_id, seq)
);

CREATE TABLE session_snapshots (
    companion_id TEXT PRIMARY KEY,
    sessions TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE launches (
    id TEXT PRIMARY KEY,
    companion_id TEXT NOT NULL,
    user TEXT NOT NULL,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    command TEXT NOT NULL,
    result TEXT,
    created_at TEXT NOT NULL,
    resulted_at TEXT
);

CREATE TABLE cap_hits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    payload TEXT NOT NULL,
    at TEXT NOT NULL
);

CREATE TABLE stale_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    companion_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    at TEXT NOT NULL
);

CREATE TABLE plans (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    branch TEXT NOT NULL,
    path TEXT NOT NULL,
    plan TEXT,
    parse_error TEXT,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (repo, number)
);

CREATE TABLE pull_requests (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    pr TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (repo, number)
);

CREATE TABLE decisions (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    comment_id INTEGER NOT NULL,
    record TEXT NOT NULL,
    PRIMARY KEY (repo, number, comment_id)
);

CREATE TABLE issue_meta (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    title TEXT,
    github_state TEXT,
    url TEXT,
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (repo, number)
);
