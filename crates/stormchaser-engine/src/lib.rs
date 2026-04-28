pub mod artifact;
pub mod db;
pub mod git_cache;
pub mod handler;
pub mod hcl_eval;
pub mod hitl;
pub mod junit;
pub mod nats;
pub mod persistence;
pub mod resource_utils;
pub mod s3;
pub mod secrets;
pub mod step_machine;
pub mod telemetry;
pub mod wasm;
pub mod workflow_machine;

use once_cell::sync::Lazy;
use opentelemetry::{
    global,
    metrics::{Counter, Histogram},
};
use std::time::Duration;

pub static RUNS_STARTED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.runs_started")
        .with_description("Total number of runs started")
        .build()
});

pub static RUNS_COMPLETED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.runs_completed")
        .with_description("Total number of runs completed successfully")
        .build()
});

pub static RUNS_FAILED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.runs_failed")
        .with_description("Total number of runs failed")
        .build()
});

pub static STEPS_STARTED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.steps_started")
        .with_description("Total number of steps started")
        .build()
});

pub static STEPS_COMPLETED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.steps_completed")
        .with_description("Total number of steps completed successfully")
        .build()
});

pub static STEPS_FAILED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .u64_counter("stormchaser.steps_failed")
        .with_description("Total number of steps failed")
        .build()
});

pub static STEP_DURATION: Lazy<Histogram<f64>> = Lazy::new(|| {
    global::meter("stormchaser-engine")
        .f64_histogram("stormchaser.step_duration_seconds")
        .with_description("Duration of step execution in seconds")
        .build()
});

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
