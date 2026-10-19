//! Ceilings on the quotas a run may give itself.
//!
//! A workflow's `quotas` block, and the timeout override of a run started through the
//! API, are chosen by whoever writes or starts the run. Without a ceiling they limit
//! nothing: a run could ask for any concurrency and run for ever. The operator sets the
//! most any run may have; a run that asks for more gets the most, and a warning is
//! logged.

use std::time::Duration;
use stormchaser_model::dsl::limits::{parse_cpu_millis, parse_memory_bytes};

/// Most steps one run may have running at once. Default 50.
pub const MAX_CONCURRENCY_ENV: &str = "STORMCHASER_RUN_MAX_CONCURRENCY";
/// Most CPU one run's steps may claim together. Default `64`.
pub const MAX_CPU_ENV: &str = "STORMCHASER_RUN_MAX_CPU";
/// Most memory one run's steps may claim together. Default `256Gi`.
pub const MAX_MEMORY_ENV: &str = "STORMCHASER_RUN_MAX_MEMORY";
/// Longest a run may last. Default `24h`.
pub const MAX_TIMEOUT_ENV: &str = "STORMCHASER_RUN_MAX_TIMEOUT";

/// The most any run may have.
#[derive(Debug, Clone, PartialEq)]
pub struct RunLimits {
    max_concurrency: u32,
    max_cpu: String,
    max_memory: String,
    max_timeout: Duration,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_concurrency: 50,
            max_cpu: "64".to_string(),
            max_memory: "256Gi".to_string(),
            max_timeout: Duration::from_secs(24 * 3600),
        }
    }
}

impl RunLimits {
    /// Reads the ceilings from the environment. A setting that is missing or cannot be
    /// read keeps its default.
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let defaults = Self::default();
        let get = |name: &str| get(name).map(|v| v.trim().to_string());
        Self {
            max_concurrency: get(MAX_CONCURRENCY_ENV)
                .and_then(|v| v.parse().ok())
                .filter(|n| *n > 0)
                .unwrap_or(defaults.max_concurrency),
            max_cpu: get(MAX_CPU_ENV)
                .filter(|v| parse_cpu_millis(v).is_some())
                .unwrap_or(defaults.max_cpu),
            max_memory: get(MAX_MEMORY_ENV)
                .filter(|v| parse_memory_bytes(v).is_some())
                .unwrap_or(defaults.max_memory),
            max_timeout: get(MAX_TIMEOUT_ENV)
                .and_then(|v| humantime::parse_duration(&v).ok())
                .filter(|d| !d.is_zero())
                .unwrap_or(defaults.max_timeout),
        }
    }

    /// `requested`, or the ceiling if it is higher.
    pub fn concurrency(&self, requested: u32) -> u32 {
        if requested > self.max_concurrency {
            tracing::warn!(
                "A run asked for concurrency {}; limited to {} ({})",
                requested,
                self.max_concurrency,
                MAX_CONCURRENCY_ENV
            );
            return self.max_concurrency;
        }
        requested
    }

    /// `requested`, or the ceiling if it is higher or cannot be read.
    pub fn cpu(&self, requested: &str) -> String {
        Self::quantity(requested, &self.max_cpu, parse_cpu_millis, MAX_CPU_ENV)
    }

    /// `requested`, or the ceiling if it is higher or cannot be read.
    pub fn memory(&self, requested: &str) -> String {
        Self::quantity(
            requested,
            &self.max_memory,
            parse_memory_bytes,
            MAX_MEMORY_ENV,
        )
    }

    /// `requested`, or the ceiling if it is longer or cannot be read.
    pub fn timeout(&self, requested: &str) -> String {
        match humantime::parse_duration(requested.trim()) {
            Ok(timeout) if timeout <= self.max_timeout => requested.to_string(),
            _ => {
                tracing::warn!(
                    "A run asked for timeout {:?}; limited to {} ({})",
                    requested,
                    humantime::format_duration(self.max_timeout),
                    MAX_TIMEOUT_ENV
                );
                humantime::format_duration(self.max_timeout).to_string()
            }
        }
    }

    fn quantity(
        requested: &str,
        ceiling: &str,
        parse: fn(&str) -> Option<u64>,
        setting: &str,
    ) -> String {
        match (parse(requested.trim()), parse(ceiling)) {
            (Some(asked), Some(most)) if asked <= most => requested.to_string(),
            _ => {
                tracing::warn!(
                    "A run asked for {:?}; limited to {} ({})",
                    requested,
                    ceiling,
                    setting
                );
                ceiling.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_gets_what_it_asks_for_up_to_the_ceiling() {
        let limits = RunLimits::default();
        assert_eq!(limits.concurrency(10), 10);
        assert_eq!(limits.concurrency(10_000), 50);
        assert_eq!(limits.cpu("2"), "2");
        assert_eq!(limits.cpu("100000"), "64");
        assert_eq!(limits.cpu("lots"), "64");
        assert_eq!(limits.memory("4Gi"), "4Gi");
        assert_eq!(limits.memory("1Ti"), "256Gi");
        assert_eq!(limits.timeout("1h"), "1h");
        assert_eq!(limits.timeout("1000d"), "1day");
        assert_eq!(limits.timeout("never"), "1day");
    }

    #[test]
    fn the_operator_sets_the_ceilings() {
        let limits = RunLimits::from_lookup(|name| {
            match name {
                MAX_CONCURRENCY_ENV => Some("5"),
                MAX_CPU_ENV => Some("4"),
                MAX_MEMORY_ENV => Some("not a size"),
                MAX_TIMEOUT_ENV => Some("2h"),
                _ => None,
            }
            .map(str::to_string)
        });
        assert_eq!(limits.concurrency(10), 5);
        assert_eq!(limits.cpu("8"), "4");
        assert_eq!(
            limits.memory("1Ti"),
            "256Gi",
            "an unreadable setting keeps the default"
        );
        assert_eq!(limits.timeout("3h"), "2h");
    }
}
