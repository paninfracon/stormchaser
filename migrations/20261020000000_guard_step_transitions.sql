-- The database owns a step's status changes: whatever statement changes a step, these
-- triggers refuse a move a step may not make, and record every change in
-- step_status_history. Before, the engine's state machine wrote history and the
-- statements that change steps in bulk (claiming, putting back, retrying) did not,
-- and nothing stopped a finished step being changed again.

-- Whether a step may move from `old` to `new`. A step that has not finished may move
-- to any status, except that only a pending step is claimed, only a running step packs
-- its storage, and only a step that was handed out can lose its runner. A finished step
-- never moves, except that a failure can be ignored afterwards (allow_failure).
CREATE FUNCTION step_status_transition_allowed(old step_status, new step_status)
RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN old = new THEN true
        WHEN old = 'failed' THEN new = 'failed_ignored'
        WHEN old IN ('succeeded', 'failed_ignored', 'skipped', 'aborted', 'lost_zombie') THEN false
        WHEN new = 'initializing' THEN old = 'pending'
        WHEN new = 'packing_sfs' THEN old = 'running'
        WHEN new = 'lost_zombie' THEN old IN ('initializing', 'unpacking_sfs', 'running', 'packing_sfs')
        ELSE true
    END
$$;

CREATE FUNCTION step_instances_guard_transition()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT step_status_transition_allowed(OLD.status, NEW.status) THEN
        RAISE EXCEPTION 'step % cannot move from % to %', OLD.id, OLD.status, NEW.status
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER step_instances_guard_transition
    BEFORE UPDATE OF status ON step_instances
    FOR EACH ROW
    EXECUTE FUNCTION step_instances_guard_transition();

CREATE FUNCTION step_instances_record_status()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'INSERT' OR NEW.status IS DISTINCT FROM OLD.status THEN
        INSERT INTO step_status_history (step_instance_id, status) VALUES (NEW.id, NEW.status);
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER step_instances_record_status
    AFTER INSERT OR UPDATE OF status ON step_instances
    FOR EACH ROW
    EXECUTE FUNCTION step_instances_record_status();
