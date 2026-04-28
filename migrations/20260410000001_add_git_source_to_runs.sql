-- Add GIT source fields to workflow_runs
ALTER TABLE workflow_runs
ADD COLUMN repo_url TEXT NOT NULL,
ADD COLUMN workflow_path TEXT NOT NULL,
ADD COLUMN git_ref TEXT NOT NULL;

-- Make workflow_name slightly more descriptive for GIT-based workflows
-- by allowing it to be derived from the file path if needed.
-- In the API request, we'll still take it explicitly or infer it.
