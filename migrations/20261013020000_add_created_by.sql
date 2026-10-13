-- Who created each connection, webhook, event rule and cron schedule: the creator's email,
-- or their subject id when the token carries no email (the same value a run records as
-- its initiating user). Policy uses it to decide who may see and change the object.
--
-- Rows that already exist keep NULL, meaning "no owner".
ALTER TABLE connections ADD COLUMN created_by TEXT;
ALTER TABLE webhooks ADD COLUMN created_by TEXT;
ALTER TABLE event_rules ADD COLUMN created_by TEXT;
ALTER TABLE cron_workflows ADD COLUMN created_by TEXT;
