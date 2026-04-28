-- Add 'resolving' and 'start_pending' to run_status enum
-- Use IF NOT EXISTS for 'resolving' in case it was partially added
-- Postgres doesn't allow ALTER TYPE ... ADD VALUE in a transaction until v12.
-- For sqlx migrations, we can try to run it.
ALTER TYPE run_status ADD VALUE IF NOT EXISTS 'resolving';
ALTER TYPE run_status ADD VALUE IF NOT EXISTS 'start_pending';
