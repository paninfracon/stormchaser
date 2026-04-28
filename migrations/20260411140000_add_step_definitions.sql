-- Registry for extensible step types and their DSL schemas
CREATE TABLE step_definitions (
    step_type TEXT PRIMARY KEY,
    schema JSONB NOT NULL, -- JSON Schema or similar describing the 'spec' block
    documentation TEXT,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Many-to-many relationship between runners and step types they support
CREATE TABLE runner_step_types (
    runner_id TEXT REFERENCES runners(id) ON DELETE CASCADE,
    step_type TEXT REFERENCES step_definitions(step_type) ON DELETE CASCADE,
    PRIMARY KEY (runner_id, step_type)
);
