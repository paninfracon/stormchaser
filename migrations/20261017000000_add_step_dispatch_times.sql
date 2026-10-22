-- When the engine claimed a step for dispatch, and when it handed the step over: its
-- task message published to a runner, or a built-in step started in the engine.
--
-- A step claimed long ago and never handed over was lost on the way, to an engine
-- that died or a publish that failed, and the engine puts it back to pending.
ALTER TABLE step_instances
    ADD COLUMN claimed_at TIMESTAMPTZ,
    ADD COLUMN dispatched_at TIMESTAMPTZ;

-- Steps already being dispatched may have been handed over; taking them for lost
-- would run them twice
UPDATE step_instances
SET claimed_at = NOW(), dispatched_at = NOW()
WHERE status = 'initializing';
