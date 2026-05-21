-- Add lost_zombie to step_status enum
ALTER TYPE step_status ADD VALUE IF NOT EXISTS 'lost_zombie';
