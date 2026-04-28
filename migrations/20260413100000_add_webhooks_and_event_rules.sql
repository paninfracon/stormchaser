-- Create webhooks table
CREATE TABLE IF NOT EXISTS webhooks (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    source_type TEXT NOT NULL, -- e.g., 'github', 'generic', 'gitlab'
    secret_token TEXT, -- HMAC secret or API key
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create event rules table
CREATE TABLE IF NOT EXISTS event_rules (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    webhook_id UUID REFERENCES webhooks(id) ON DELETE CASCADE,
    event_type_pattern TEXT NOT NULL, -- e.g., 'pull_request.*' or 'push'
    condition_expr TEXT, -- CEL expression to filter the payload
    workflow_name TEXT NOT NULL,
    repo_url TEXT NOT NULL,
    workflow_path TEXT NOT NULL,
    git_ref TEXT NOT NULL DEFAULT 'main',
    input_mappings JSONB NOT NULL DEFAULT '{}', -- Map of input_name -> CEL expression
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Trigger to update updated_at
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_webhooks_updated_at BEFORE UPDATE ON webhooks FOR EACH ROW EXECUTE PROCEDURE update_updated_at_column();
CREATE TRIGGER update_event_rules_updated_at BEFORE UPDATE ON event_rules FOR EACH ROW EXECUTE PROCEDURE update_updated_at_column();
