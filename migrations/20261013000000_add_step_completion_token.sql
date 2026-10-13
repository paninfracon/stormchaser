-- A per-step secret that must accompany the event completing or failing a step the
-- engine handles itself (approvals, waits, built-in steps). It is cleared when a step is
-- dispatched to a runner and never leaves the database for a runner, so a runner
-- credential cannot complete an approval.
--
-- Added in two statements on purpose: rows that already exist keep NULL (no token
-- required), so steps in flight during the upgrade can still finish.
ALTER TABLE step_instances ADD COLUMN completion_token TEXT;
ALTER TABLE step_instances ALTER COLUMN completion_token SET DEFAULT gen_random_uuid()::text;
