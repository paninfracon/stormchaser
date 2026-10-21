-- Whether a step holds its share of its run's CPU and memory quota. Set when dispatch
-- claims the quota and cleared when it is given back, so that each claim is given back
-- once, whichever of a failed dispatch, the step's result or lost-dispatch recovery
-- gets there first.
ALTER TABLE step_instances
    ADD COLUMN quota_held BOOLEAN NOT NULL DEFAULT false;

-- Steps out with a runner when this runs claimed theirs under the old accounting
UPDATE step_instances
SET quota_held = true
WHERE status IN ('initializing', 'running', 'unpacking_sfs', 'packing_sfs');
