-- Migration to add storage and artifact backends configuration

CREATE TYPE backend_type AS ENUM ('s3', 'oci', 'jfrog', 'gcs', 'azure');

CREATE TABLE storage_backends (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    backend_type backend_type NOT NULL,
    config JSONB NOT NULL, -- e.g., { "bucket": "...", "endpoint": "...", "region": "..." }
    is_default_sfs BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Ensure only one default SFS backend exists (optional but recommended)
CREATE UNIQUE INDEX idx_only_one_default_sfs ON storage_backends (is_default_sfs) WHERE (is_default_sfs = TRUE);

-- Table for artifact specifications (linking storage to workflows)
CREATE TABLE artifact_registry (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    step_instance_id UUID NOT NULL REFERENCES step_instances(id) ON DELETE CASCADE,
    artifact_name TEXT NOT NULL,
    backend_id UUID NOT NULL REFERENCES storage_backends(id),
    remote_path TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_artifact_registry_run_id ON artifact_registry(run_id);

CREATE TABLE run_storage_states (
    run_id UUID NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    storage_name TEXT NOT NULL,
    last_hash TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (run_id, storage_name)
);
