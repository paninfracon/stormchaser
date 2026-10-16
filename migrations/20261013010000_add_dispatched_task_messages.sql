-- Where each task message sent to a runner sits in the NATS stream.
--
-- A task message carries the step's resolved spec, secrets included. The engine uses
-- this table to erase the message from the stream once the step is finished or its run
-- archived, so secrets do not stay readable through NATS for the life of the stream.
--
-- There is no foreign key to step_instances on purpose: archiving deletes the step row,
-- and the message still has to be erased afterwards.
CREATE TABLE dispatched_task_messages (
    stream_seq BIGINT PRIMARY KEY,
    step_instance_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_dispatched_task_messages_step ON dispatched_task_messages(step_instance_id);
