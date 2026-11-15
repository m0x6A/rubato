-- Issue #122, PR 3 (ADR 0051). Messages a person sends from Rubato to a session they launched.
-- Owner: slice session_messages. Additive. A row is keyed by launch; session_id is only the
-- launch's current session, which a `/clear` changes. `state` is where the message is (queued,
-- sent, delivered, refused, failed, delivery_unknown); a row that is delivery_unknown is never
-- pushed again. `kind` and `body` hold the protocol's SessionMessageKind.
CREATE TABLE session_messages (
    id           TEXT PRIMARY KEY NOT NULL,
    launch_id    TEXT NOT NULL,
    session_id   TEXT,
    companion_id TEXT NOT NULL,
    owner        TEXT NOT NULL,
    repo         TEXT NOT NULL,
    number       INTEGER NOT NULL,
    kind         TEXT NOT NULL,
    body         TEXT NOT NULL,
    author       TEXT NOT NULL,
    priority     INTEGER NOT NULL,
    state        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    pushed_at    TEXT,
    outcome_at   TEXT,
    detail       TEXT
);

CREATE INDEX session_messages_by_launch ON session_messages (launch_id, state);
CREATE INDEX session_messages_by_companion ON session_messages (companion_id, state);
