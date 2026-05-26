-- Add initializing to step_status enum
ALTER TYPE step_status ADD VALUE IF NOT EXISTS 'initializing';
