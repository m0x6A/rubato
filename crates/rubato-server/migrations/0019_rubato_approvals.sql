-- Issue #122, PR 6 (ADR 0054, plan 122 design §D6). Additive: one table. Owner: approve.
--
-- An approval given in Rubato: one person approved one revision of a draft, as themselves. One
-- row per version (repo, number, branch, plan_sha, bundle_hash): a second press of the same version finds the
-- first row. `self` is 1 when the approver owns the draft. `summary` is the discussion summary as
-- the approver edited it. `handoff_outcome` and `handoff_at` are facts from the handoff report
-- (PR 7), not a state; whether an approval is current is derived from the draft and the plan on
-- the branch, never stored.
CREATE TABLE rubato_approvals (
    id              TEXT PRIMARY KEY NOT NULL,
    repo            TEXT NOT NULL,
    number          INTEGER NOT NULL,
    branch          TEXT NOT NULL,
    draft_id        TEXT NOT NULL,
    rev             INTEGER NOT NULL,
    plan_sha        TEXT NOT NULL,
    bundle_hash     TEXT NOT NULL,
    approved_by     TEXT NOT NULL,
    approved_at     TEXT NOT NULL,
    self            INTEGER NOT NULL,
    summary         TEXT NOT NULL,
    handoff_outcome TEXT,
    handoff_at      TEXT,
    created_at      TEXT NOT NULL,
    UNIQUE (repo, number, branch, plan_sha, bundle_hash)
);
