//! What a runner lets one step use, whatever its workflow asks for.
//!
//! A step's CPU, memory, run time and parallelism come from its workflow, and a step
//! that states none of them used to get none: no memory ceiling, no time limit. On a
//! runner that several teams share, one workflow could then starve the host or the
//! namespace for everyone. The operator sets these limits on the runner; a step that
//! asks for more is refused, and a step that asks for nothing gets the defaults.

use anyhow::{bail, Result};
use std::time::Duration;

/// CPU a step gets when it asks for none, in Kubernetes notation. Default `1`.
pub const DEFAULT_CPU_ENV: &str = "STORMCHASER_STEP_DEFAULT_CPU";
/// Memory a step gets when it asks for none. Default `2Gi`.
pub const DEFAULT_MEMORY_ENV: &str = "STORMCHASER_STEP_DEFAULT_MEMORY";
/// Most CPU a step may ask for. Default `8`.
pub const MAX_CPU_ENV: &str = "STORMCHASER_STEP_MAX_CPU";
/// Most memory a step may ask for. Default `16Gi`.
pub const MAX_MEMORY_ENV: &str = "STORMCHASER_STEP_MAX_MEMORY";
/// Time a step may run when it sets no `timeout`. Default `24h`.
pub const DEFAULT_TIMEOUT_ENV: &str = "STORMCHASER_STEP_DEFAULT_TIMEOUT";
/// Longest `timeout` a step may set. Default `24h`.
pub const MAX_TIMEOUT_ENV: &str = "STORMCHASER_STEP_MAX_TIMEOUT";
/// Most processes and threads a step's container may have (Docker runner). Default 4096.
pub const PIDS_LIMIT_ENV: &str = "STORMCHASER_STEP_PIDS_LIMIT";
/// Most pods one step's Job may run at once, or ask to complete (Kubernetes runner).
/// Default 10.
pub const MAX_PARALLELISM_ENV: &str = "STORMCHASER_STEP_MAX_PARALLELISM";

/// The limits a runner applies to every step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepLimits {
    /// CPU for a step that asks for none.
    pub default_cpu: String,
    /// Memory for a step that asks for none.
    pub default_memory: String,
    /// Most CPU a step may ask for, in millicores.
    pub max_cpu_millis: u64,
    /// Most memory a step may ask for, in bytes.
    pub max_memory_bytes: u64,
    /// Run time for a step that sets no timeout.
    pub default_timeout: Duration,
    /// Longest run time a step may set.
    pub max_timeout: Duration,
    /// Most processes a step's container may have.
    pub pids_limit: i64,
    /// Most pods one step may run at once or ask to complete.
    pub max_parallelism: i32,
}

impl Default for StepLimits {
    fn default() -> Self {
        Self {
            default_cpu: "1".to_string(),
            default_memory: "2Gi".to_string(),
            max_cpu_millis: 8_000,
            max_memory_bytes: 16 * 1024 * 1024 * 1024,
            default_timeout: Duration::from_secs(24 * 3600),
            max_timeout: Duration::from_secs(24 * 3600),
            pids_limit: 4096,
            max_parallelism: 10,
        }
    }
}

/// CPU and memory for one step, after the runner's limits have been applied.
#[derive(Debug, Clone, PartialEq)]
pub struct StepResources {
    /// CPU in Kubernetes notation, as the step or the default wrote it.
    pub cpu: String,
    /// Memory in Kubernetes notation.
    pub memory: String,
    /// The same CPU in millicores.
    pub cpu_millis: u64,
    /// The same memory in bytes.
    pub memory_bytes: u64,
}

impl StepLimits {
    /// Reads the limits from the runner's environment. A setting that is missing or
    /// cannot be read falls back to its default, so that a typing mistake never leaves a
    /// runner without limits.
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let defaults = Self::default();
        let quantity = |name: &str, parse: fn(&str) -> Option<u64>, fallback: u64| {
            get(name).and_then(|v| parse(v.trim())).unwrap_or(fallback)
        };
        let duration = |name: &str, fallback: Duration| {
            get(name)
                .and_then(|v| humantime::parse_duration(v.trim()).ok())
                .filter(|d| !d.is_zero())
                .unwrap_or(fallback)
        };
        let max_timeout = duration(MAX_TIMEOUT_ENV, defaults.max_timeout);
        Self {
            default_cpu: get(DEFAULT_CPU_ENV)
                .filter(|v| parse_cpu_millis(v.trim()).is_some())
                .map(|v| v.trim().to_string())
                .unwrap_or(defaults.default_cpu),
            default_memory: get(DEFAULT_MEMORY_ENV)
                .filter(|v| parse_memory_bytes(v.trim()).is_some())
                .map(|v| v.trim().to_string())
                .unwrap_or(defaults.default_memory),
            max_cpu_millis: quantity(MAX_CPU_ENV, parse_cpu_millis, defaults.max_cpu_millis),
            max_memory_bytes: quantity(
                MAX_MEMORY_ENV,
                parse_memory_bytes,
                defaults.max_memory_bytes,
            ),
            default_timeout: duration(DEFAULT_TIMEOUT_ENV, defaults.default_timeout)
                .min(max_timeout),
            max_timeout,
            pids_limit: get(PIDS_LIMIT_ENV)
                .and_then(|v| v.trim().parse().ok())
                .filter(|v| *v > 0)
                .unwrap_or(defaults.pids_limit),
            max_parallelism: get(MAX_PARALLELISM_ENV)
                .and_then(|v| v.trim().parse().ok())
                .filter(|v| *v > 0)
                .unwrap_or(defaults.max_parallelism),
        }
    }

    /// The CPU and memory a step gets: what it asked for if that is within the limits,
    /// the defaults where it asked for nothing.
    pub fn resources(&self, cpu: Option<&str>, memory: Option<&str>) -> Result<StepResources> {
        let cpu = cpu.map(str::trim).filter(|v| !v.is_empty());
        let memory = memory.map(str::trim).filter(|v| !v.is_empty());

        let cpu = cpu.unwrap_or(&self.default_cpu).to_string();
        let Some(cpu_millis) = parse_cpu_millis(&cpu) else {
            bail!("Step asks for CPU '{}', which is not a CPU quantity", cpu);
        };
        if cpu_millis == 0 || cpu_millis > self.max_cpu_millis {
            bail!(
                "Step asks for CPU '{}'; this runner allows up to {}m ({})",
                cpu,
                self.max_cpu_millis,
                MAX_CPU_ENV
            );
        }

        let memory = memory.unwrap_or(&self.default_memory).to_string();
        let Some(memory_bytes) = parse_memory_bytes(&memory) else {
            bail!(
                "Step asks for memory '{}', which is not a memory quantity",
                memory
            );
        };
        if memory_bytes == 0 || memory_bytes > self.max_memory_bytes {
            bail!(
                "Step asks for memory '{}'; this runner allows up to {} bytes ({})",
                memory,
                self.max_memory_bytes,
                MAX_MEMORY_ENV
            );
        }

        Ok(StepResources {
            cpu,
            memory,
            cpu_millis,
            memory_bytes,
        })
    }

    /// How long a step may run: its own `timeout` if that is within the limit, the
    /// default if it sets none.
    pub fn timeout(&self, requested: Option<&str>) -> Result<Duration> {
        let Some(requested) = requested.map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(self.default_timeout);
        };
        let Ok(timeout) = humantime::parse_duration(requested) else {
            bail!("Step timeout '{}' is not a duration", requested);
        };
        if timeout.is_zero() || timeout > self.max_timeout {
            bail!(
                "Step timeout '{}' is outside what this runner allows: up to {} ({})",
                requested,
                humantime::format_duration(self.max_timeout),
                MAX_TIMEOUT_ENV
            );
        }
        Ok(timeout)
    }

    /// Refuses a Job that would run, or ask for, more pods than the runner allows.
    pub fn check_parallelism(
        &self,
        parallelism: Option<i32>,
        completions: Option<i32>,
    ) -> Result<()> {
        for (what, value) in [("parallelism", parallelism), ("completions", completions)] {
            if let Some(value) = value {
                if value < 1 || value > self.max_parallelism {
                    bail!(
                        "Step sets {} to {}; this runner allows 1 to {} ({})",
                        what,
                        value,
                        self.max_parallelism,
                        MAX_PARALLELISM_ENV
                    );
                }
            }
        }
        Ok(())
    }
}

/// Reads a CPU quantity (`500m`, `2`, `0.5`) as millicores.
pub fn parse_cpu_millis(cpu: &str) -> Option<u64> {
    let millis = match cpu.strip_suffix('m') {
        Some(millis) => millis.parse::<f64>().ok()?,
        None => cpu.parse::<f64>().ok()? * 1000.0,
    };
    (0.0..=u32::MAX as f64)
        .contains(&millis)
        .then(|| millis.round() as u64)
}

/// Reads a memory quantity (`512Mi`, `2Gi`, `1G`, `1048576`) as bytes.
pub fn parse_memory_bytes(memory: &str) -> Option<u64> {
    const UNITS: &[(&str, u64)] = &[
        ("Ki", 1 << 10),
        ("Mi", 1 << 20),
        ("Gi", 1 << 30),
        ("Ti", 1 << 40),
        ("k", 1_000),
        ("K", 1_000),
        ("M", 1_000_000),
        ("G", 1_000_000_000),
        ("T", 1_000_000_000_000),
    ];
    for (suffix, factor) in UNITS {
        if let Some(number) = memory.strip_suffix(suffix) {
            let number = number.parse::<f64>().ok()?;
            let bytes = number * *factor as f64;
            return (0.0..1e18).contains(&bytes).then(|| bytes.round() as u64);
        }
    }
    memory.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_are_read_as_kubernetes_writes_them() {
        assert_eq!(parse_cpu_millis("500m"), Some(500));
        assert_eq!(parse_cpu_millis("2"), Some(2000));
        assert_eq!(parse_cpu_millis("0.5"), Some(500));
        assert_eq!(parse_cpu_millis("lots"), None);
        assert_eq!(parse_cpu_millis("-1"), None);
        assert_eq!(parse_memory_bytes("512Mi"), Some(512 << 20));
        assert_eq!(parse_memory_bytes("2Gi"), Some(2 << 30));
        assert_eq!(parse_memory_bytes("1G"), Some(1_000_000_000));
        assert_eq!(parse_memory_bytes("1048576"), Some(1 << 20));
        assert_eq!(parse_memory_bytes("1.5Gi"), Some(3 << 29));
        assert_eq!(parse_memory_bytes("huge"), None);
    }

    #[test]
    fn a_step_that_asks_for_nothing_gets_the_defaults_not_everything() {
        let limits = StepLimits::default();
        let resources = limits.resources(None, Some("  ")).unwrap();
        assert_eq!(resources.cpu, "1");
        assert_eq!(resources.memory, "2Gi");
        assert_eq!(resources.cpu_millis, 1000);
        assert_eq!(resources.memory_bytes, 2 << 30);
        assert_eq!(
            limits.timeout(None).unwrap(),
            Duration::from_secs(24 * 3600)
        );
    }

    #[test]
    fn a_step_that_asks_for_too_much_is_refused() {
        let limits = StepLimits::default();
        assert!(limits.resources(Some("8"), Some("16Gi")).is_ok());
        assert!(limits.resources(Some("8001m"), None).is_err());
        assert!(limits.resources(None, Some("17Gi")).is_err());
        assert!(limits.resources(Some("0"), None).is_err());
        assert!(limits.resources(Some("many"), None).is_err());

        assert_eq!(
            limits.timeout(Some("30m")).unwrap(),
            Duration::from_secs(1800)
        );
        assert!(limits.timeout(Some("25h")).is_err());
        assert!(limits.timeout(Some("0s")).is_err());
        assert!(limits.timeout(Some("soon")).is_err());

        assert!(limits.check_parallelism(Some(10), Some(1)).is_ok());
        assert!(limits.check_parallelism(Some(11), None).is_err());
        assert!(limits.check_parallelism(None, Some(1000)).is_err());
        assert!(limits.check_parallelism(Some(0), None).is_err());
    }

    #[test]
    fn the_operator_sets_the_limits_and_a_mistake_falls_back_to_the_default() {
        let env = |name: &str| match name {
            MAX_CPU_ENV => Some("2".to_string()),
            MAX_MEMORY_ENV => Some("4Gi".to_string()),
            DEFAULT_MEMORY_ENV => Some("512Mi".to_string()),
            MAX_TIMEOUT_ENV => Some("2h".to_string()),
            // Longer than the maximum: held to it
            DEFAULT_TIMEOUT_ENV => Some("5h".to_string()),
            PIDS_LIMIT_ENV => Some("not a number".to_string()),
            _ => None,
        };
        let limits = StepLimits::from_lookup(env);
        assert_eq!(limits.max_cpu_millis, 2000);
        assert_eq!(limits.max_memory_bytes, 4 << 30);
        assert_eq!(limits.default_memory, "512Mi");
        assert_eq!(limits.max_timeout, Duration::from_secs(7200));
        assert_eq!(limits.default_timeout, Duration::from_secs(7200));
        assert_eq!(limits.pids_limit, StepLimits::default().pids_limit);
        assert!(limits.resources(Some("3"), None).is_err());
    }
}
