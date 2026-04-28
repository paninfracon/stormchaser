-- Add initiating_user to workflow_runs to track who started the workflow
ALTER TABLE workflow_runs ADD COLUMN initiating_user TEXT NOT NULL DEFAULT 'system';
