-- Issue #122, PR 11 (ADR 0057, plan 122 design §D8). Additive. Owner: rooms.
--
-- An agent called into a plan's room, by its launch (a session id changes on /clear; the launch
-- does not). A row is a stay: joining again after leaving makes a new row. While left_at is null
-- the agent is in; at most one agent per owner is in a room at once, since its handle is
-- `<owner>-agent`. last_turn_at is when it was last addressed: a turn carries the room since then
-- (or since joined_at), and an agent not addressed for Limits.room_agent_idle_secs leaves.
-- fork_session_id is the read-only fork the companion resumes for the next building-time turn.
CREATE TABLE room_agents (
    id              TEXT PRIMARY KEY NOT NULL,
    repo            TEXT NOT NULL,
    number          INTEGER NOT NULL,
    branch          TEXT NOT NULL,
    launch_id       TEXT NOT NULL,
    owner           TEXT NOT NULL,
    called_by       TEXT NOT NULL,
    joined_at       TEXT NOT NULL,
    left_at         TEXT,
    left_reason     TEXT,             -- dismissed, timed_out or session_ended
    last_turn_at    TEXT,
    fork_session_id TEXT
);

CREATE UNIQUE INDEX room_agents_one_per_owner
    ON room_agents (repo, number, branch, owner COLLATE NOCASE)
    WHERE left_at IS NULL;

-- A teammate's request to call someone else's agent in: pending until the owner accepts or
-- declines, or its asker withdraws it.
CREATE TABLE room_requests (
    id           TEXT PRIMARY KEY NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    branch       TEXT NOT NULL,
    owner        TEXT NOT NULL,
    launch_id    TEXT NOT NULL,
    requested_by TEXT NOT NULL,
    at           TEXT NOT NULL,
    state        TEXT NOT NULL,       -- pending, accepted, declined or withdrawn
    answered_at  TEXT
);

CREATE INDEX room_requests_by_room ON room_requests (repo, number, branch, state);

-- The agent handles a message addresses (JSON array), and the turn it made or answers.
ALTER TABLE room_messages ADD COLUMN addressed TEXT;
ALTER TABLE room_messages ADD COLUMN turn_id TEXT;

-- One reply per agent and turn: a replayed Stop or RoomReply posts nothing twice.
CREATE UNIQUE INDEX room_messages_one_reply
    ON room_messages (turn_id, author)
    WHERE turn_id IS NOT NULL AND author_kind = 'agent';
