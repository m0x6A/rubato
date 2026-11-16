-- Issue #146 (ADR 0073, plan 146 design §7). Additive: one column. Owner: rooms.
--
-- A room message posted from a push-to-talk utterance is `spoken`; a turn it makes is asked
-- aloud, and only an agent's reply to such a turn is ever sent to the speech provider. No audio
-- is stored: the transcript is the message text, as for a typed post.
ALTER TABLE room_messages ADD COLUMN spoken INTEGER NOT NULL DEFAULT 0;
