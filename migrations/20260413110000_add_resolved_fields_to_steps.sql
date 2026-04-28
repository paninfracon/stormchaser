-- Add resolved spec and params to step_instances to avoid re-evaluation during batching
ALTER TABLE step_instances ADD COLUMN spec JSONB NOT NULL DEFAULT '{}';
ALTER TABLE step_instances ADD COLUMN params JSONB NOT NULL DEFAULT '{}';
