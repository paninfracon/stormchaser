//! Core execution and orchestration engine for Stormchaser workflows.
//!
//! This crate contains the logic to execute steps, evaluate HCL expressions,
//! handle state transitions, and manage telemetry.

/// Artifact management and backend integrations.
pub mod artifact;
/// Database interaction layer.
pub mod db;
/// Git caching and cloning utilities.
pub mod git_cache;
/// Event handling and dispatching.
pub mod handler;
/// HCL expression evaluation and context building.
pub mod hcl_eval;
/// Human-In-The-Loop (HITL) manual approval handling.
pub mod hitl;
/// JUnit test report parsing and aggregation.
pub mod junit;
/// NATS JetStream integration for pub/sub events.
pub mod nats;
/// Database persistence for workflow and step models.
pub mod persistence;
/// Utilities for parsing resources like CPU and Memory.
pub mod resource_utils;
/// S3 backend storage implementation.
pub mod s3;
/// Secrets management interfaces and backends.
pub mod secrets;
/// State machine for individual steps.
pub mod step_machine;
/// OpenTelemetry tracing and metrics initialization.
pub mod telemetry;
/// WebAssembly module execution utilities.
pub mod wasm;
/// State machine for full workflow runs.
pub mod workflow_machine;

use once_cell::sync::Lazy;
use opentelemetry::{
    global,
    metrics::{Counter, Histogram},
};
use std::time::Duration;

/// Counter metric for the total number of started workflow runs.
pub static RUNS_STARTED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.runs_started")
        .with_description("Total number of runs started")
        .build()
});

/// Counter metric for the total number of successfully completed workflow runs.
pub static RUNS_COMPLETED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.runs_completed")
        .with_description("Total number of runs completed successfully")
        .build()
});

/// Counter metric for the total number of failed workflow runs.
pub static RUNS_FAILED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.runs_failed")
        .with_description("Total number of runs failed")
        .build()
});

/// Counter metric for the total number of started workflow steps.
pub static STEPS_STARTED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.steps_started")
        .with_description("Total number of steps started")
        .build()
});

/// Counter metric for the total number of successfully completed workflow steps.
pub static STEPS_COMPLETED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.steps_completed")
        .with_description("Total number of steps completed successfully")
        .build()
});

/// Counter metric for the total number of failed workflow steps.
pub static STEPS_FAILED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.steps_failed")
        .with_description("Total number of steps failed")
        .build()
});

/// Histogram metric for the duration of step executions in seconds.
pub static STEP_DURATION: Lazy<Histogram<f64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .f64_histogram("stormchaser.step_duration_seconds")
        .with_description("Duration of step execution in seconds")
        .build()
});

/// Parse duration.
pub fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    Ok(humantime::parse_duration(s)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("10s").unwrap(), Duration::from_secs(10));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("2h 30m").unwrap(), Duration::from_secs(9000));
    }
}
