-- 1. Remove duplicate active rows that have already been successfully archived.
-- This resolves the race conditions and flashing in the UI caused by the `UNION ALL` returning the old 'running' row.
DELETE FROM workflow_runs
WHERE id IN (SELECT id FROM archived_workflow_runs);

-- 2. Mark any remaining 'running', 'queued', 'start_pending', or 'resolving' active runs
-- as 'failed' (timeout/orphaned) if they have not been updated in the last 24 hours.
-- This ensures the system does not leak active state for crashed runners.
UPDATE workflow_runs
SET
    status = 'failed',
    error = 'Orphaned run: workflow engine crashed or was killed before reaching terminal state or archival',
    updated_at = NOW(),
    finished_at = NOW()
WHERE
    status IN ('running', 'queued', 'start_pending', 'resolving')
    AND updated_at < NOW() - INTERVAL '24 hours';
