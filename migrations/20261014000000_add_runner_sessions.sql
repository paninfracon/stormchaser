-- The key the engine issued each runner when it started. Runners sign what they report
-- with it, and the engine checks the signature, so one runner cannot report on another
-- runner's steps or mark it offline. Like every credential in this database, it relies
-- on the database being encrypted at rest.
CREATE TABLE runner_sessions (
    runner_id TEXT PRIMARY KEY,
    session_key BYTEA NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
