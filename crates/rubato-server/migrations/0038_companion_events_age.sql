-- Issue #345 follow-up: `companion_events` is pruned by age (it is the replay dedupe key and
-- nothing else, ADR 0012), so a paired companion cannot grow the database without bound. The
-- index is what the sweep reads.
CREATE INDEX companion_events_received_at ON companion_events (received_at);
