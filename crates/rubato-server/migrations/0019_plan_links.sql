-- Issue #122, PR 7 (ADR 0055, plan 122 design §D7). Additive.
--
-- plan_links (owner: approve): the link comment posted on the issue, as the approver, once the
-- handoff commit is on the branch. One row per approval: the primary key is the approval, and the
-- commit is a column, so a later push while the row is `opening` cannot make a second row or a
-- second comment. `opening` means sent with no answer (or not sent yet); it becomes `posted`
-- when GitHub answers or when the comments pass finds the `Rubato-Approval: <id>` line from the
-- approver. It is never posted again on its own.
CREATE TABLE plan_links (
    approval_id TEXT PRIMARY KEY NOT NULL,
    commit_sha  TEXT NOT NULL,
    state       TEXT NOT NULL,      -- opening or posted
    comment_id  INTEGER,
    url         TEXT,
    created_at  TEXT NOT NULL,
    posted_at   TEXT
);

-- The handoff report's facts on the approval (owner: approve): the commit it made and the ADRs it
-- could not accept (finding AdrsNotAccepted), beside handoff_outcome and handoff_at.
ALTER TABLE rubato_approvals ADD COLUMN handoff_commit TEXT;
ALTER TABLE rubato_approvals ADD COLUMN handoff_adrs_left TEXT;

-- The commit the plan was read at (owner: plans), so a permalink names exactly the text checked.
ALTER TABLE plans ADD COLUMN head_sha TEXT;
