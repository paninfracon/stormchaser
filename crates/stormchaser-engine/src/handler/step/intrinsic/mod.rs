//! Intrinsic step types that are built into the Stormchaser engine. The engine runs
//! them itself; `dispatch::intrinsic` chooses which.

/// Built-in git checkout operations.
pub mod git_checkout;
/// JQ JSON processing step.
pub mod jq;
/// REST API step.
pub mod rest_api;
/// SQL Execution step.
pub mod sql_execute;
/// Terraform step orchestration.
pub mod terraform;
/// WebAssembly module execution step.
pub mod wasm;
