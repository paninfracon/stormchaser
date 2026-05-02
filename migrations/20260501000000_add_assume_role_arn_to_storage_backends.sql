-- Add aws_assume_role_arn to storage_backends
ALTER TABLE storage_backends ADD COLUMN aws_assume_role_arn VARCHAR;
