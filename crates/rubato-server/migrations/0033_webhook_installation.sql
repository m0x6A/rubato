-- Issue #345 (ADR 0087; design §5.4). Additive, owned by `webhooks`: the App installation a
-- delivery came from, so the poll loop can apply `installation` and `installation_repositories`
-- events to that installation's tenant. NULL on rows stored before.
ALTER TABLE webhook_deliveries ADD COLUMN installation_id INTEGER;
