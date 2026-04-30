/// Parse cpu.
pub fn parse_cpu(cpu_str: &str) -> Option<f64> {
    if let Some(m_idx) = cpu_str.find('m') {
        if let Ok(m_cores) = cpu_str[..m_idx].parse::<f64>() {
            return Some(m_cores / 1000.0);
        }
    } else if let Ok(cores) = cpu_str.parse::<f64>() {
        return Some(cores);
    }
    None
}

/// Parse memory.
pub fn parse_memory(memory_str: &str) -> Option<i64> {
    let mem = memory_str.to_lowercase();
    if let Some(idx) = mem.find(|c: char| c.is_alphabetic()) {
        let val = mem[..idx].trim().parse::<i64>().ok()?;
        let suffix = &mem[idx..];
        match suffix {
            "k" | "ki" => Some(val * 1024),
            "m" | "mi" => Some(val * 1024 * 1024),
            "g" | "gi" => Some(val * 1024 * 1024 * 1024),
            "t" | "ti" => Some(val * 1024 * 1024 * 1024 * 1024),
            _ => None,
        }
    } else {
        mem.trim().parse::<i64>().ok()
    }
}

use serde_json::Value;

/// Extracts CPU (in cores) and memory (in bytes) requirements from a step's specification.
pub fn get_step_resource_requirements(step_type: &str, spec: &Value) -> (f64, i64) {
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

    (cpu_req, mem_req)
}
