-- Add WASM fields to step_definitions for orchestrator-local steps
ALTER TABLE step_definitions ADD COLUMN wasm_module TEXT;
ALTER TABLE step_definitions ADD COLUMN wasm_function TEXT;
ALTER TABLE step_definitions ADD COLUMN wasm_config JSONB;
