use super::*;
use crate::job_machine::JobMetadata;
use chrono::Utc;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateTerminated, ContainerStatus, PodStatus,
};
use std::collections::HashMap;
use stormchaser_model::dsl;
use stormchaser_model::dsl::Step;
use uuid::Uuid;

#[test]
fn test_do_extract_pod_metrics() {
    let pod = Pod {
        status: Some(PodStatus {
            container_statuses: Some(vec![ContainerStatus {
                restart_count: 2,
                state: Some(ContainerState {
                    terminated: Some(ContainerStateTerminated {
                        exit_code: 1,
                        reason: Some("Error".to_string()),
                        message: Some("Something went wrong".to_string()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let (exit_code, attempts, reason) = do_extract_pod_metrics_with_reason(vec![pod]);
    assert_eq!(exit_code, Some(1));
    assert_eq!(attempts, 3);
    assert_eq!(reason, Some("Error: Something went wrong".to_string()));
}

#[test]
fn test_do_build_job_spec_basic() {
    let step_dsl = Step {
        name: "test-step".into(),
        r#type: "RunContainer".into(),
        params: HashMap::new(),
        spec: serde_json::json!({
            "image": "alpine:latest",
            "command": ["echo"],
            "args": ["hello"]
        }),
        aggregation: vec![],
        next: vec![],
        outputs: vec![],
        reports: vec![],
        condition: None,
        strategy: None,
        iterate: None,
        iterate_as: None,
        steps: None,
        on_failure: None,
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        artifacts: None,
    };

    let metadata = JobMetadata {
        run_id: Uuid::new_v4(),
        step_id: Uuid::new_v4(),
        step_dsl,
        namespace: "default".into(),
        received_at: Utc::now(),
        cluster_version: "v1.28.0".into(),
        encryption_key: None,
        storage: None,
        test_report_urls: None,
        registry_auth: None,
    };

    let job = do_build_job_spec("test-job", &metadata, None, None).unwrap();
    assert_eq!(job.metadata.name, Some("test-job".into()));
    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod_spec.containers[0].image, Some("alpine:latest".into()));
}

#[test]
fn test_do_build_job_spec_with_pvc() {
    let run_id = Uuid::new_v4();
    let step_dsl = Step {
        name: "test-pvc-step".into(),
        r#type: "RunContainer".into(),
        params: HashMap::new(),
        spec: serde_json::json!({
            "image": "alpine:latest",
            "command": ["echo"],
            "args": ["hello"],
            "storage_mounts": [
                {
                    "name": "workspace",
                    "mount_path": "/workspace",
                    "read_only": false
                }
            ]
        }),
        aggregation: vec![],
        next: vec![],
        outputs: vec![],
        reports: vec![],
        condition: None,
        strategy: None,
        iterate: None,
        iterate_as: None,
        steps: None,
        on_failure: None,
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        artifacts: None,
    };

    let mut storage_map = HashMap::new();
    storage_map.insert(
        "workspace".to_string(),
        serde_json::json!({
            "put_url": "http://example.com/put",
            "get_url": "http://example.com/get",
            "expected_hash": "abcd",
            "provision": [
                {
                    "url": "http://example.com/provision.tar.gz",
                    "destination": "/"
                }
            ]
        }),
    );

    let metadata = JobMetadata {
        run_id,
        step_id: Uuid::new_v4(),
        step_dsl,
        namespace: "default".into(),
        received_at: Utc::now(),
        cluster_version: "v1.28.0".into(),
        encryption_key: None,
        storage: Some(storage_map),
        test_report_urls: None,
        registry_auth: None,
    };

    let job = do_build_job_spec(
        "test-job-pvc",
        &metadata,
        None,
        Some("my-shared-pvc".to_string()),
    )
    .unwrap();

    let pod_spec = job.spec.unwrap().template.spec.unwrap();

    // Should have the pvc volume
    let vol = pod_spec
        .volumes
        .as_ref()
        .unwrap()
        .iter()
        .find(|v| v.name == "sfs-workspace")
        .unwrap();
    assert!(vol.persistent_volume_claim.is_some());
    assert_eq!(
        vol.persistent_volume_claim.as_ref().unwrap().claim_name,
        "my-shared-pvc"
    );

    // Main container should have the volume mount with sub_path
    let main_container = &pod_spec.containers[0];
    let mount = main_container
        .volume_mounts
        .as_ref()
        .unwrap()
        .iter()
        .find(|m| m.name == "sfs-workspace")
        .unwrap();
    assert_eq!(mount.sub_path, Some(format!("{}/workspace", run_id)));

    // Because we are using PVC, it should NOT inject STORMCHASER_PUT_URL_workspace for S3
    let env = main_container.env.as_ref().unwrap();
    assert!(!env
        .iter()
        .any(|e| e.name == "STORMCHASER_PUT_URL_workspace"));

    // But because of provision, it SHOULD have a provision init container
    assert!(pod_spec.init_containers.is_some());
    let init_containers = pod_spec.init_containers.as_ref().unwrap();
    let prov_container = init_containers
        .iter()
        .find(|c| c.name == "prov-workspace-0");
    assert!(prov_container.is_some());

    // The unpark container (from get_url/expected_hash) should NOT be present
    let unpark_container = init_containers
        .iter()
        .find(|c| c.name == "unpark-workspace");
    assert!(unpark_container.is_none());
}

#[test]
fn test_normalize_resource_name() {
    assert_eq!(
        normalize_resource_name("My_Resource", "prefix"),
        "prefix-my-resource"
    );
    assert_eq!(
        normalize_resource_name("Another-Test", "job"),
        "job-another-test"
    );
}

#[test]
fn test_map_dsl_env_to_k8s() {
    let dsl_env = vec![
        dsl::EnvVar {
            name: "KEY1".to_string(),
            value: "VAL1".to_string(),
        },
        dsl::EnvVar {
            name: "KEY2".to_string(),
            value: "VAL2".to_string(),
        },
    ];
    let k8s_env = map_dsl_env_to_k8s(dsl_env);
    assert_eq!(k8s_env.len(), 2);
    assert_eq!(k8s_env[0].name, "KEY1");
    assert_eq!(k8s_env[0].value, Some("VAL1".to_string()));
}

#[test]
fn test_build_k8s_resources() {
    let res = build_k8s_resources(Some("500m".to_string()), Some("1Gi".to_string()));
    let requests = res.requests.unwrap();
    let limits = res.limits.unwrap();
    assert_eq!(requests["cpu"].0, "500m");
    assert_eq!(requests["memory"].0, "1Gi");
    assert_eq!(limits["cpu"].0, "500m");
    assert_eq!(limits["memory"].0, "1Gi");
}
