-- Issue #122, PR 10 (ADR 0057, plan 122 design §D8). Additive: one table. Owner: rooms.
--
-- A plan's room is keyed like its draft, (repo, number, branch). People post; Rubato posts a
-- system notice when this plan and another open plan or settled draft touch the same seam. The
-- notice_* columns name the other issue, its branch and the seam; they are null for a person's
-- message. The agent columns of design §D8 (agent_session, addressed, turn_id) come with PR 11.
CREATE TABLE room_messages (
    id            TEXT PRIMARY KEY NOT NULL,
    repo          TEXT NOT NULL,
    number        INTEGER NOT NULL,
    branch        TEXT NOT NULL,
    author_kind   TEXT NOT NULL,      -- person, agent or system
    author        TEXT NOT NULL,
    text          TEXT NOT NULL,
    at            TEXT NOT NULL,
    notice_seam   TEXT,
    notice_repo   TEXT,
    notice_number INTEGER,
    notice_branch TEXT
);

CREATE INDEX room_messages_by_room ON room_messages (repo, number, branch, at);

-- One seam notice per room, other issue and seam: a second tick, or the other plan moving to
-- another branch, posts no second one.
CREATE UNIQUE INDEX room_messages_one_notice
    ON room_messages (repo, number, branch, notice_repo, notice_number, notice_seam)
    WHERE notice_seam IS NOT NULL;
