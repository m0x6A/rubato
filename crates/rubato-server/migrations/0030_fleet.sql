-- Issue #134 (ADR 0072, plan 134 design §5). Additive. Behind the orchestrator flag, off by
-- default: with it off nothing writes these.
--
-- Owner fleet: one row per lead fleet request, claimed (state in_progress) before it acts, keyed
-- by the companion, the lead launch and the request id, with a hash of the action so the same
-- id with another action is refused. Rows older than 30 days are deleted on insert.
CREATE TABLE fleet_requests (
    companion_id     TEXT NOT NULL,
    lead_launch_id   TEXT NOT NULL,
    request_id       TEXT NOT NULL,
    action_hash      TEXT NOT NULL,
    action           TEXT NOT NULL,   -- list, launch, message, stop
    owner            TEXT NOT NULL,
    target_launch_id TEXT,
    state            TEXT NOT NULL,   -- in_progress, done
    outcome          TEXT,
    reason           TEXT,
    result           TEXT,            -- the FleetResult JSON, for a repeat
    claimed_at       TEXT NOT NULL,
    done_at          TEXT,
    PRIMARY KEY (companion_id, lead_launch_id, request_id)
);

CREATE INDEX fleet_requests_by_age ON fleet_requests (claimed_at);

-- Owner launch: a launch's role and the lead that asked for it, written from the command.
-- Old rows are workers started by a person; both stay null for them.
ALTER TABLE launches ADD COLUMN role TEXT;
ALTER TABLE launches ADD COLUMN lead_launch_id TEXT;
CREATE INDEX launches_by_lead ON launches (lead_launch_id);
CREATE INDEX launches_by_user ON launches (user, created_at);

-- Owner session_messages: the lead agent that sent a chat for its owner, shown as via lead.
ALTER TABLE session_messages ADD COLUMN via_launch TEXT;
