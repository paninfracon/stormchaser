-- Steps waiting when completion tokens were added kept NULL, which the engine reads as
-- "a runner has it", so the approval or wait could then not be completed by the API or
-- the engine, which name no runner. A waiting step has not been handed to a runner yet
-- (dispatch clears the token when it is), so each gets a token now.
UPDATE step_instances
SET completion_token = gen_random_uuid()::text
WHERE completion_token IS NULL
  AND status = 'waiting_for_event'
  AND runner_id IS NULL;
