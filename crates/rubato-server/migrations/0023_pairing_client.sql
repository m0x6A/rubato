-- Issue #138 (#122 PR 13, ADR 0062). Additive, owned by `pairing`: what kind of client each
-- companion said it is in its last Hello, so its lane shows *headless* even while it is offline.
-- 0021 is reserved for #122 PR 11 (room call-ins).
ALTER TABLE pairings ADD COLUMN client TEXT NOT NULL DEFAULT 'interactive';
