-- The column holds a connection's secret as written. It was named encrypted_credentials,
-- but Stormchaser does not encrypt it; encryption at rest is the database's.
ALTER TABLE connections RENAME COLUMN encrypted_credentials TO credentials;
