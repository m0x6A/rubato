-- Issue #80, PR 3 (ADR 0030). Additive: one table. Owner: auth.

-- A person's GitHub App user-to-server tokens, encrypted with AES-256-GCM under
-- RUBATO_TOKEN_KEY: hex(nonce || ciphertext). Kept only as an App with expiring user tokens.
CREATE TABLE user_tokens (
    login               TEXT PRIMARY KEY,
    access_ct           TEXT NOT NULL,
    refresh_ct          TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    refresh_expires_at  TEXT,
    updated_at          TEXT NOT NULL
);
