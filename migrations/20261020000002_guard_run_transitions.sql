-- The database refuses a status change the state machines do not make, for runs as
-- well as steps, and holds steps to the moves in docs/state_machines.md rather than
-- letting an unfinished step move to almost any status.

-- Whether a run may move from `old` to `new`. A run that has not finished may fail or
-- be aborted (timed out) from any status, and otherwise only moves forward; a
-- finished run never moves.
CREATE FUNCTION run_status_transition_allowed(old run_status, new run_status)
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN old = new THEN true
        WHEN old IN ('succeeded', 'failed', 'aborted') THEN false
        WHEN new IN ('failed', 'aborted') THEN true
        WHEN old = 'queued' THEN new = 'resolving'
        WHEN old = 'resolving' THEN new = 'start_pending'
        WHEN old = 'start_pending' THEN new = 'running'
        WHEN old = 'running' THEN new = 'succeeded'
        ELSE false
    END
$$;

CREATE FUNCTION workflow_runs_guard_transition()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT run_status_transition_allowed(OLD.status, NEW.status) THEN
        RAISE EXCEPTION 'run % cannot move from % to %', OLD.id, OLD.status, NEW.status
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER workflow_runs_guard_transition
    BEFORE UPDATE OF status ON workflow_runs
    FOR EACH ROW
    EXECUTE FUNCTION workflow_runs_guard_transition();

-- Whether a step may move from `old` to `new`, as the step machine moves it. A step's
-- result can overtake the reports of its progress, so a step that has been handed
-- out may finish from any status it has in flight. A step in flight goes back to
-- pending when it is put back for a retry, or when its claim or its engine is lost.
CREATE OR REPLACE FUNCTION step_status_transition_allowed(old step_status, new step_status)
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN old = new THEN true
        WHEN old = 'pending' THEN new IN ('initializing', 'unpacking_sfs', 'running', 'failed')
        WHEN old = 'initializing' THEN
            new IN ('pending', 'unpacking_sfs', 'running', 'succeeded', 'failed', 'lost_zombie')
        WHEN old = 'unpacking_sfs' THEN
            new IN ('pending', 'running', 'succeeded', 'failed', 'lost_zombie')
        WHEN old = 'running' THEN
            new IN ('pending', 'packing_sfs', 'succeeded', 'failed', 'lost_zombie')
        WHEN old = 'packing_sfs' THEN new IN ('pending', 'succeeded', 'failed', 'lost_zombie')
        WHEN old = 'waiting_for_event' THEN new IN ('pending', 'succeeded', 'failed')
        WHEN old = 'failed' THEN new = 'failed_ignored'
        ELSE false
    END
$$;
