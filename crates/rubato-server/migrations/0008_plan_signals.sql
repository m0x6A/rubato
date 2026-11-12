-- Issue #71, PR 5 (ADR 0027). Additive: one column, one table.

-- The approval hash of the plan file as read: checkboxes cleared, git blob sha1. Owner: plans.
ALTER TABLE plans ADD COLUMN approval_sha TEXT;

-- `Review:` and `Approved:` comments from members, one row per comment. Owner: github_poll.
CREATE TABLE plan_signals (
    repo        TEXT NOT NULL,
    number      INTEGER NOT NULL,
    comment_id  INTEGER NOT NULL,
    kind        TEXT NOT NULL,          -- review or approval
    record      TEXT NOT NULL,          -- ReviewRecord or ApprovalRecord as JSON
    target_sha  TEXT,                   -- approval on another branch: that branch's plan hash
    PRIMARY KEY (repo, number, comment_id)
);
