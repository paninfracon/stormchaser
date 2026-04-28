-- Add secrets column to run_contexts and archived_run_contexts
ALTER TABLE run_contexts ADD COLUMN IF NOT EXISTS secrets JSONB NOT NULL DEFAULT '{}';
ALTER TABLE archived_run_contexts ADD COLUMN IF NOT EXISTS secrets JSONB NOT NULL DEFAULT '{}';
