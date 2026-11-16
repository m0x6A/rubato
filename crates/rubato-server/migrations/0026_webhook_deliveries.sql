-- Issue #208 (ADR 0074). Additive, owned by `webhooks`: one row per verified GitHub App webhook
-- delivery, with what it names and no payload, kept 7 days. 0024 and 0025 are reserved for #144
-- and #82.
CREATE TABLE webhook_deliveries (
    delivery_id  TEXT PRIMARY KEY,
    body_sha256  TEXT NOT NULL UNIQUE,
    event        TEXT NOT NULL,
    action       TEXT,
    repo         TEXT,
    issue_number INTEGER,
    branch       TEXT,
    -- Unix seconds.
    received_at  INTEGER NOT NULL,
    -- `accepted` (names something to refresh) or `ignored`.
    outcome      TEXT NOT NULL,
    attempts     INTEGER NOT NULL DEFAULT 0,
    -- Unix seconds; NULL while what it names is not read yet.
    refreshed_at INTEGER
);
CREATE INDEX webhook_deliveries_pending ON webhook_deliveries (received_at) WHERE refreshed_at IS NULL;
CREATE INDEX webhook_deliveries_received ON webhook_deliveries (received_at);
