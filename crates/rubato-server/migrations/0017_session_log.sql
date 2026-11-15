-- Issue #122, PR 4 (ADR 0052). A launched session's conversation, as the team sees it: the
-- prompt of each turn (UserPromptSubmit) and its last reply (Stop), joined by Claude Code's
-- prompt_id. Owner: slice session_log. Additive. Kept without pruning (plan 122 A11).
CREATE TABLE session_log_entries (
    session_id TEXT NOT NULL,
    prompt_id  TEXT NOT NULL,
    role       TEXT NOT NULL,
    launch_id  TEXT NOT NULL,
    repo       TEXT NOT NULL,
    number     INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    text       TEXT NOT NULL,
    truncated  INTEGER NOT NULL,
    message_id TEXT,
    mixed      INTEGER NOT NULL,
    at         TEXT NOT NULL,
    PRIMARY KEY (session_id, prompt_id, role)
);

CREATE INDEX session_log_by_launch ON session_log_entries (launch_id, at);
