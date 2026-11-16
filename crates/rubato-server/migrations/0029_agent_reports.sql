-- What each launched session's agent last said it is doing (ADR 0071, plan 226 §2), owned by
-- slice agent_reports. One row per launch; only the companion the launch went to changes it.
-- Additive: dropping the table loses nothing else.
CREATE TABLE agent_reports (
    launch_id    TEXT PRIMARY KEY,
    companion_id TEXT NOT NULL,
    doing        TEXT NOT NULL,
    parts        TEXT NOT NULL,
    reported_at  TEXT NOT NULL
);
CREATE INDEX agent_reports_companion ON agent_reports (companion_id);
