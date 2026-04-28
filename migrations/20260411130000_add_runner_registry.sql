-- Create runner registry for live step runners
CREATE TYPE runner_status AS ENUM ('online', 'offline');

CREATE TABLE runners (
    id TEXT PRIMARY KEY,
    runner_type TEXT NOT NULL,
    status runner_status NOT NULL DEFAULT 'online',
    protocol_version TEXT NOT NULL,
    capabilities TEXT[] NOT NULL DEFAULT '{}',
    nats_subject TEXT NOT NULL,
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_runners_type ON runners(runner_type);
CREATE INDEX idx_runners_status ON runners(status);
