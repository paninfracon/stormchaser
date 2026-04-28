-- Add source_code column to run_contexts to store the original workflow file
ALTER TABLE run_contexts ADD COLUMN source_code TEXT NOT NULL DEFAULT '';
