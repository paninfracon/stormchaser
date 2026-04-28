-- Migration to add cron workflows support

CREATE TABLE IF NOT EXISTS cron_workflows (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    cronspec TEXT NOT NULL,
    workflow_name TEXT NOT NULL,
    repo_url TEXT NOT NULL,
    workflow_path TEXT NOT NULL,
    git_ref TEXT NOT NULL DEFAULT 'main',
    inputs JSONB NOT NULL DEFAULT '{}',
    secret_token TEXT NOT NULL, -- Token required to trigger the webhook
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    external_job_id TEXT, -- ID in the external cron engine
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Trigger to update updated_at
CREATE TRIGGER update_cron_workflows_updated_at BEFORE UPDATE ON cron_workflows FOR EACH ROW EXECUTE PROCEDURE update_updated_at_column();
