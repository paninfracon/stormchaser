use k8s_openapi::api::core::v1::ResourceRequirements as K8sResources;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use std::collections::BTreeMap;

pub(crate) fn build_k8s_resources(cpu: Option<String>, memory: Option<String>) -> K8sResources {
    let mut requests = BTreeMap::new();
    let mut limits = BTreeMap::new();
    if let Some(cpu) = cpu {
        requests.insert("cpu".to_string(), Quantity(cpu.clone()));
        limits.insert("cpu".to_string(), Quantity(cpu));
    }
    if let Some(mem) = memory {
        requests.insert("memory".to_string(), Quantity(mem.clone()));
        limits.insert("memory".to_string(), Quantity(mem));
    }
    K8sResources {
        requests: Some(requests),
        limits: Some(limits),
        ..Default::default()
    }
}

/// Holds a step's CPU and memory to the runner's limits.
///
/// The limit for each is what the step set as its limit, or failing that its request, or
/// failing that the runner's default; it is refused if it is over the runner's maximum.
/// A request the step set is kept if it does not exceed the limit. Other resources the
/// step names are passed through as written.
pub(crate) fn apply_step_limits(
    mut resources: K8sResources,
    limits: &stormchaser_model::dsl::limits::StepLimits,
) -> anyhow::Result<K8sResources> {
    use stormchaser_model::dsl::limits::{parse_cpu_millis, parse_memory_bytes};

    let wanted = |name: &str| -> Option<String> {
        resources
            .limits
            .as_ref()
            .and_then(|l| l.get(name))
            .or_else(|| resources.requests.as_ref().and_then(|r| r.get(name)))
            .map(|q| q.0.clone())
    };
    let granted = limits.resources(wanted("cpu").as_deref(), wanted("memory").as_deref())?;

    let request_limits = resources.limits.get_or_insert_with(BTreeMap::new);
    request_limits.insert("cpu".to_string(), Quantity(granted.cpu.clone()));
    request_limits.insert("memory".to_string(), Quantity(granted.memory.clone()));

    let requests = resources.requests.get_or_insert_with(BTreeMap::new);
    let cpu_request_fits = requests
        .get("cpu")
        .and_then(|q| parse_cpu_millis(&q.0))
        .is_some_and(|millis| millis <= granted.cpu_millis);
    if !cpu_request_fits {
        requests.insert("cpu".to_string(), Quantity(granted.cpu));
    }
    let memory_request_fits = requests
        .get("memory")
        .and_then(|q| parse_memory_bytes(&q.0))
        .is_some_and(|bytes| bytes <= granted.memory_bytes);
    if !memory_request_fits {
        requests.insert("memory".to_string(), Quantity(granted.memory));
    }
    Ok(resources)
}
