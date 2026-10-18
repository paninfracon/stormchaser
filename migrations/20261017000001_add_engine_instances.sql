-- Each engine process records a heartbeat, and a built-in step records the engine
-- process that runs it. A built-in step whose engine has stopped sending heartbeats
-- will not finish, and the engine fails it, as it does a step whose runner has gone.
CREATE TABLE engine_instances (
    id UUID PRIMARY KEY,
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE step_instances ADD COLUMN engine_id UUID;
