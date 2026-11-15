-- Issue #122, PR 1 (ADR 0052). Hook events were stored with Claude Code's raw payload, which
-- carries tool input and output: file contents, command output, prompts. Nothing reads it; the
-- table exists for replay deduplication by (companion_id, seq) (companion_link/store.rs). From
-- now on the companion forwards only an allowlist and the server stores no payload at all.
--
-- Not reversible, on purpose: this is data that should never have left the developer's machine.
-- Rollback of the deploy: restore the database copy taken before it, with the previous sha.
UPDATE companion_events
   SET payload = json_remove(payload, '$.payload')
 WHERE kind = 'hook'
   AND json_type(payload, '$.payload') IS NOT NULL;
