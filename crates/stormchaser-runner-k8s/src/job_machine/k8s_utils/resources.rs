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
