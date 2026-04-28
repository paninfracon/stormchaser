-- Migration to add mTLS support for storage and webhook backends

-- Add TLS certificate and key columns to storage_backends
ALTER TABLE storage_backends ADD COLUMN ca_cert TEXT;
ALTER TABLE storage_backends ADD COLUMN client_cert TEXT;
ALTER TABLE storage_backends ADD COLUMN client_key TEXT;

-- Add TLS certificate and key columns to webhooks
ALTER TABLE webhooks ADD COLUMN ca_cert TEXT;
ALTER TABLE webhooks ADD COLUMN client_cert TEXT;
ALTER TABLE webhooks ADD COLUMN client_key TEXT;

-- For other 3rd party endpoints, we might have a generic external_endpoints table
-- But for now we just add it to the existing tables that connect to external services.
