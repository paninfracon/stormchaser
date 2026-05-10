ALTER TYPE backend_type RENAME TO connection_type;
ALTER TYPE connection_type ADD VALUE 'postgres';
ALTER TYPE connection_type ADD VALUE 'mysql';
ALTER TYPE connection_type ADD VALUE 'http_api';
ALTER TYPE connection_type ADD VALUE 'git';

ALTER TABLE storage_backends RENAME TO connections;
ALTER TABLE connections ADD COLUMN encrypted_credentials TEXT;

ALTER TABLE artifact_registry RENAME COLUMN backend_id TO connection_id;
ALTER TABLE step_test_reports RENAME COLUMN backend_id TO connection_id;

ALTER INDEX idx_only_one_default_sfs RENAME TO idx_only_one_default_sfs_connection;
