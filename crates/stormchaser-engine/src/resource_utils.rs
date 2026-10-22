use stormchaser_model::dsl::limits::{parse_cpu_millis, parse_memory_bytes};

/// Reads a CPU quantity (`500m`, `2`, `0.5`) as cores, the way runners read it.
pub fn parse_cpu(cpu_str: &str) -> Option<f64> {
    parse_cpu_millis(cpu_str.trim()).map(|millis| millis as f64 / 1000.0)
}

/// Reads a memory quantity (`512Mi`, `1.5Gi`, `1G`) as bytes, the way runners read it.
pub fn parse_memory(memory_str: &str) -> Option<i64> {
    parse_memory_bytes(memory_str.trim()).and_then(|bytes| i64::try_from(bytes).ok())
}

use serde_json::Value;

pub struct StepResourceRequirements {
    pub cpu: f64,
    pub memory: i64,
}

/// Extracts CPU (in cores) and memory (in bytes) requirements from a step's specification.
pub fn get_step_resource_requirements(step_type: &str, spec: &Value) -> StepResourceRequirements {
    let mut cpu_req = 0.0;
    let mut mem_req = 0;

    if step_type == "RunContainer" || step_type == "RunK8sJob" {
        if let Some(cpu) = spec.get("cpu").and_then(|v| v.as_str()) {
            cpu_req = parse_cpu(cpu).unwrap_or(0.0);
        }
        if let Some(mem) = spec.get("memory").and_then(|v| v.as_str()) {
            mem_req = parse_memory(mem).unwrap_or(0);
        }

        if step_type == "RunK8sJob" {
            // Also check resources block for K8sJobSpec
            if let Some(resources) = spec.get("resources") {
                if let Some(requests) = resources.get("requests") {
                    if let Some(cpu) = requests.get("cpu").and_then(|v| v.as_str()) {
                        cpu_req = parse_cpu(cpu).unwrap_or(cpu_req);
                    }
                    if let Some(mem) = requests.get("memory").and_then(|v| v.as_str()) {
                        mem_req = parse_memory(mem).unwrap_or(mem_req);
                    }
                }
            }
        }
    }

    StepResourceRequirements {
        cpu: cpu_req,
        memory: mem_req,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quotas_read_quantities_as_runners_do() {
        assert_eq!(parse_cpu("500m"), Some(0.5));
        assert_eq!(parse_cpu("2"), Some(2.0));
        // Fractions and decimal units, which this parser used to read as nothing or as
        // binary units
        assert_eq!(parse_memory("1.5Gi"), Some(3 << 29));
        assert_eq!(parse_memory("1G"), Some(1_000_000_000));
        assert_eq!(parse_memory("512Mi"), Some(512 << 20));
        assert_eq!(parse_memory("lots"), None);

        let req = get_step_resource_requirements(
            "RunContainer",
            &json!({ "cpu": "250m", "memory": "1.5Gi" }),
        );
        assert_eq!((req.cpu, req.memory), (0.25, 3 << 29));
    }
}
