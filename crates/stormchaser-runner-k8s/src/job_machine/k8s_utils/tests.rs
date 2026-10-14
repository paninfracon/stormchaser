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

    let metrics = do_extract_pod_metrics_with_reason(vec![pod]);
    assert_eq!(metrics.exit_code, Some(1));
    assert_eq!(metrics.attempts, 3);
    assert_eq!(
        metrics.failure_reason,
        Some("Error: Something went wrong".to_string())
    );
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
        aliases: std::collections::HashMap::new(),
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
        fencing_token: 0,
        step_dsl,
        namespace: "default".into(),
        received_at: Utc::now(),
        cluster_version: "v1.28.0".into(),
        encryption_key: None,
        storage: None,
        test_report_urls: None,
        registry_auth: None,
        runner_id: None,
    };

    let (job, _secrets) = do_build_job_spec("test-job", &metadata, None, None).unwrap();
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
        aliases: std::collections::HashMap::new(),
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
        fencing_token: 0,
        step_dsl,
        namespace: "default".into(),
        received_at: Utc::now(),
        cluster_version: "v1.28.0".into(),
        encryption_key: None,
        storage: Some(storage_map),
        test_report_urls: None,
        registry_auth: None,
        runner_id: None,
    };

    let (job, _secrets) = do_build_job_spec(
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

    // The step's container never gets upload URLs
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

#[test]
fn test_do_build_job_spec_rejects_privileged_by_default() {
    for step_type in ["RunContainer", "RunK8sJob"] {
        let step_dsl = Step {
            name: "test-step".into(),
            r#type: step_type.into(),
            params: HashMap::new(),
            spec: serde_json::json!({
                "image": "alpine:latest",
                "privileged": true
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
            aliases: std::collections::HashMap::new(),
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
            fencing_token: 0,
            step_dsl,
            namespace: "default".into(),
            received_at: Utc::now(),
            cluster_version: "v1.28.0".into(),
            encryption_key: None,
            storage: None,
            test_report_urls: None,
            registry_auth: None,
            runner_id: None,
        };

        let err = do_build_job_spec("test-job", &metadata, None, None).unwrap_err();
        assert!(
            format!("{:#}", err).contains(dsl::ALLOW_PRIVILEGED_ENV),
            "{} should be rejected",
            step_type
        );
    }
}

fn k8s_job_metadata(spec: serde_json::Value) -> JobMetadata {
    let step_dsl = Step {
        name: "test-step".into(),
        r#type: "RunK8sJob".into(),
        params: HashMap::new(),
        spec,
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
        aliases: std::collections::HashMap::new(),
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        artifacts: None,
    };

    JobMetadata {
        run_id: Uuid::new_v4(),
        step_id: Uuid::new_v4(),
        fencing_token: 7,
        step_dsl,
        namespace: "default".into(),
        received_at: Utc::now(),
        cluster_version: "v1.28.0".into(),
        encryption_key: None,
        storage: None,
        test_report_urls: None,
        registry_auth: None,
        runner_id: None,
    }
}

#[test]
fn test_do_build_job_spec_rejects_unlisted_cluster_objects() {
    let cases = [
        (
            serde_json::json!({ "image": "alpine", "service_account_name": "runner-sa" }),
            ALLOWED_SERVICE_ACCOUNTS_ENV,
        ),
        (
            serde_json::json!({
                "image": "alpine",
                "secret_mounts": [{ "name": "postgres-credentials", "mount_path": "/x" }]
            }),
            ALLOWED_SECRET_MOUNTS_ENV,
        ),
        (
            serde_json::json!({
                "image": "alpine",
                "config_map_mounts": [{ "name": "opa-policy", "mount_path": "/x" }]
            }),
            ALLOWED_CONFIG_MAP_MOUNTS_ENV,
        ),
    ];

    for (spec, env_name) in cases {
        let metadata = k8s_job_metadata(spec);
        let err = do_build_job_spec("test-job", &metadata, None, None).unwrap_err();
        assert!(format!("{:#}", err).contains(env_name));
    }
}

#[test]
fn test_step_policy_allow_list_matching() {
    let policy = StepPolicy {
        allowed_service_accounts: vec!["deployer".into()],
        allowed_secret_mounts: vec!["workflow-*".into()],
        allowed_config_map_mounts: vec![],
    };

    let allowed = parse_step_spec(&k8s_job_metadata(serde_json::json!({
        "image": "alpine",
        "service_account_name": "deployer",
        "secret_mounts": [{ "name": "workflow-npm-token", "mount_path": "/x" }]
    })))
    .unwrap();
    assert!(policy.check(&allowed).is_ok());

    let wrong_prefix = parse_step_spec(&k8s_job_metadata(serde_json::json!({
        "image": "alpine",
        "secret_mounts": [{ "name": "postgres-workflow-x", "mount_path": "/x" }]
    })))
    .unwrap();
    assert!(policy.check(&wrong_prefix).is_err());
}

#[test]
fn test_do_build_job_spec_withholds_api_token_and_protects_reserved_labels() {
    let victim_run = Uuid::new_v4().to_string();
    let metadata = k8s_job_metadata(serde_json::json!({
        "image": "alpine",
        "labels": { "managed-by": "someone-else", "stormchaser-run-id": victim_run, "team": "a" },
        "annotations": { "stormchaser.v1.io/state-encrypted": "true", "note": "b" }
    }));

    let (job, _secrets) = do_build_job_spec("test-job", &metadata, None, None).unwrap();

    let labels = job.metadata.labels.unwrap();
    assert_eq!(labels["managed-by"], "stormchaser");
    assert_eq!(labels["stormchaser-run-id"], metadata.run_id.to_string());
    assert_eq!(labels["team"], "a");

    let annotations = job.metadata.annotations.unwrap();
    assert!(!annotations.contains_key("stormchaser.v1.io/state-encrypted"));
    assert_eq!(annotations["note"], "b");

    let pod_spec = job.spec.unwrap().template.spec.unwrap();
    assert_eq!(pod_spec.automount_service_account_token, Some(false));
    assert_eq!(pod_spec.service_account_name, None);
}

fn storage_step_metadata(storage: serde_json::Value, reports: bool) -> JobMetadata {
    let mut metadata = k8s_job_metadata(serde_json::json!({
        "image": "alpine:latest",
        "command": ["make", "build"],
        "env": [{"name": "MODE", "value": "release"}],
        "storage_mounts": [{"name": "workspace", "mount_path": "/workspace"}]
    }));
    metadata.storage = Some(HashMap::from([("workspace".to_string(), storage)]));
    if reports {
        metadata.step_dsl.reports = vec![stormchaser_model::dsl::TestReport {
            name: "unit".into(),
            path: "out/*.xml".into(),
            format: "junit".into(),
        }];
        metadata.test_report_urls = Some(HashMap::from([(
            "unit".to_string(),
            serde_json::json!({"put_url": "https://s3.example.com/report-put"}),
        )]));
    }
    metadata
}

fn oci_storage() -> serde_json::Value {
    serde_json::json!({
        "put_url": "https://s3.example.com/SECRET-PUT-URL",
        "get_url": null,
        "artifacts": {
            "app": {
                "connection_type": "oci",
                "remote_path": "registry.example.com/run/workspace:app",
                "path": "bin/app",
                "needs_auth": true
            }
        }
    })
}

#[test]
fn storage_is_parked_by_a_sidecar_and_the_step_container_holds_nothing_to_upload_with() {
    let metadata = storage_step_metadata(oci_storage(), false);
    let (job, secrets) =
        do_build_job_spec("storm-build-1", &metadata, Some("agent:1".into()), None).unwrap();
    let template = job.spec.unwrap().template;
    let pod = template.spec.unwrap();

    assert_eq!(pod.containers.len(), 2);
    let worker = &pod.containers[0];
    let sidecar = &pod.containers[1];
    assert_eq!(worker.name, "worker");
    assert_eq!(sidecar.name, "stormchaser-agent");

    // The step runs the author's command and nothing of the agent's
    let worker_text = serde_json::to_string(worker).unwrap();
    assert!(!worker_text.contains("SECRET-PUT-URL"));
    assert!(!worker_text.contains("stormchaser-agent"));
    assert!(!worker_text.contains("registry.example.com"));
    let worker_mounts: Vec<&str> = worker
        .volume_mounts
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.mount_path.as_str())
        .collect();
    assert_eq!(worker_mounts, vec!["/workspace"]);
    assert!(pod
        .init_containers
        .unwrap()
        .iter()
        .all(|c| c.name != "inject-agent"));

    // The sidecar shares the storage volume, waits for the runner, and does the uploads
    let args = sidecar.args.as_ref().unwrap();
    assert_eq!(args[0], "park");
    assert!(args.contains(&"/stormchaser/control/worker-exit".to_string()));
    // Its upload instructions are not in the pod: the arguments name environment
    // variables, which read the step's Secret
    assert!(args.contains(&"$(STORMCHASER_PARKING_URLS)".to_string()));
    assert!(args.contains(&"$(STORMCHASER_ARTIFACT_URLS)".to_string()));
    assert!(!serde_json::to_string(sidecar)
        .unwrap()
        .contains("SECRET-PUT-URL"));
    let sidecar_env = sidecar.env.as_ref().unwrap();
    assert!(sidecar_env.iter().all(|var| var.value.is_none()
        && var
            .value_from
            .as_ref()
            .and_then(|from| from.secret_key_ref.as_ref())
            .is_some_and(|key| key.name == "storm-build-1-env")));
    assert!(secrets
        .values()
        .iter()
        .any(|v| v.contains("SECRET-PUT-URL")));
    // Artifact paths are relative to the volume they belong to
    assert!(secrets
        .values()
        .iter()
        .any(|v| v.contains(r#""path":"/workspace/bin/app""#)));
    assert!(args.contains(&"/stormchaser/auth/config.json".to_string()));
    let sidecar_mounts: Vec<&str> = sidecar
        .volume_mounts
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.mount_path.as_str())
        .collect();
    assert_eq!(
        sidecar_mounts,
        vec!["/workspace", "/stormchaser/control", "/stormchaser/auth"]
    );

    // The runner's signal, and credentials that do not exist until the step is done
    assert_eq!(
        template.metadata.unwrap().annotations.unwrap()["stormchaser.v1.io/worker-exit"],
        ""
    );
    let volumes = pod.volumes.unwrap();
    let auth = volumes
        .iter()
        .find(|v| v.name == "storm-push-auth")
        .unwrap();
    let secret = auth.secret.as_ref().unwrap();
    assert_eq!(
        secret.secret_name.as_deref(),
        Some("storm-build-1-push-auth")
    );
    assert_eq!(secret.optional, Some(true));
    assert!(volumes
        .iter()
        .any(|v| v.name == "storm-control" && v.downward_api.is_some()));
}

#[test]
fn no_sidecar_without_an_agent_image_or_with_nothing_to_park() {
    // No agent image configured
    let metadata = storage_step_metadata(oci_storage(), false);
    let (job, _secrets) = do_build_job_spec("j", &metadata, None, None).unwrap();
    let template = job.spec.unwrap().template;
    assert_eq!(template.spec.unwrap().containers.len(), 1);
    assert!(template.metadata.unwrap().annotations.is_none());

    // A shared PVC keeps the volume by itself, and there are no artifacts
    let metadata = storage_step_metadata(
        serde_json::json!({"put_url": "https://s3.example.com/put"}),
        false,
    );
    let (job, _secrets) =
        do_build_job_spec("j", &metadata, Some("agent:1".into()), Some("pvc".into())).unwrap();
    assert_eq!(job.spec.unwrap().template.spec.unwrap().containers.len(), 1);
}

#[test]
fn test_reports_are_still_collected_inside_the_step_container() {
    let metadata = storage_step_metadata(oci_storage(), true);
    let (job, _secrets) = do_build_job_spec("j", &metadata, Some("agent:1".into()), None).unwrap();
    let pod = job.spec.unwrap().template.spec.unwrap();
    let worker = &pod.containers[0];

    // The agent wraps the command there, with report settings only
    let script = &worker.args.as_ref().unwrap()[1];
    assert!(script.contains("stormchaser-agent run --parking-urls '{}' --mount-paths '{}'"));
    let env: Vec<&str> = worker
        .env
        .as_ref()
        .unwrap()
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert!(env.contains(&"STORMCHASER_TEST_REPORTS"));
    assert!(env.contains(&"STORMCHASER_REPORT_URLS"));
    assert!(!serde_json::to_string(worker)
        .unwrap()
        .contains("SECRET-PUT-URL"));
    assert!(pod
        .init_containers
        .unwrap()
        .iter()
        .any(|c| c.name == "inject-agent"));
    assert_eq!(pod.containers.len(), 2);
}

#[test]
fn storage_mount_paths_that_cover_a_helper_are_refused() {
    let mut metadata = storage_step_metadata(oci_storage(), false);
    metadata.step_dsl.spec["storage_mounts"][0]["mount_path"] =
        serde_json::json!("/stormchaser/auth");
    let err = do_build_job_spec("j", &metadata, Some("agent:1".into()), None).unwrap_err();
    assert!(err.to_string().contains("reserved"), "{err}");
}

#[test]
fn the_steps_exit_code_is_its_own_containers() {
    let pod = |worker: i32, sidecar: i32| -> k8s_openapi::api::core::v1::Pod {
        serde_json::from_value(serde_json::json!({
            "status": {"containerStatuses": [
                {"name": "stormchaser-agent", "ready": false, "restartCount": 0, "image": "", "imageID": "",
                 "state": {"terminated": {"exitCode": sidecar}}},
                {"name": "worker", "ready": false, "restartCount": 0, "image": "", "imageID": "",
                 "state": {"terminated": {"exitCode": worker}}}
            ]}
        }))
        .unwrap()
    };
    assert_eq!(
        do_extract_pod_metrics_with_reason(vec![pod(0, 0)]).exit_code,
        Some(0)
    );
    assert_eq!(
        do_extract_pod_metrics_with_reason(vec![pod(3, 3)]).exit_code,
        Some(3)
    );
    assert_eq!(
        do_extract_pod_metrics_with_reason(vec![pod(3, 0)]).exit_code,
        Some(3)
    );
    // Parking failed after the step succeeded
    assert_eq!(
        do_extract_pod_metrics_with_reason(vec![pod(0, 1)]).exit_code,
        Some(1)
    );
}

#[test]
fn nothing_secret_is_written_into_the_job() {
    let mut metadata = k8s_job_metadata(serde_json::json!({
        "image": "alpine:latest",
        "command": ["make", "deploy"],
        "env": [
            {"name": "DB_PASSWORD", "value": "hunter2-env-secret"},
            {"name": "STORMCHASER_CONN_DB_PASSWORD", "value": "injected-connection-secret"},
            {"name": "EMPTY", "value": ""}
        ],
        "storage_mounts": [{"name": "workspace", "mount_path": "/workspace"}]
    }));
    metadata
        .step_dsl
        .params
        .insert("token".into(), "param-secret".into());
    metadata.storage = Some(HashMap::from([(
        "workspace".to_string(),
        serde_json::json!({
            "put_url": "https://s3.example.com/PRESIGNED-PUT",
            "get_url": "https://s3.example.com/PRESIGNED-GET",
            "expected_hash": "abc"
        }),
    )]));
    metadata.encryption_key = None;

    let (job, secrets) =
        do_build_job_spec("storm-deploy-1", &metadata, Some("agent:1".into()), None).unwrap();

    // The whole Job, as anyone who may read Jobs or pods would see it
    let visible = serde_json::to_string(&job).unwrap();
    for secret in [
        "hunter2-env-secret",
        "injected-connection-secret",
        "param-secret",
        "PRESIGNED-PUT",
        "PRESIGNED-GET",
    ] {
        assert!(!visible.contains(secret), "{secret} is readable in the Job");
        assert!(
            secrets.values().iter().any(|v| v.contains(secret)) || secret == "param-secret",
            "{secret} was dropped instead of being moved to the Secret"
        );
    }
    // Names stay readable, so the pod can still be understood
    assert!(visible.contains("DB_PASSWORD"));

    // The step's container reads each value from the Secret
    let pod = job.spec.as_ref().unwrap().template.spec.as_ref().unwrap();
    let worker_env = pod.containers[0].env.as_ref().unwrap();
    for name in ["DB_PASSWORD", "STORMCHASER_CONN_DB_PASSWORD", "EMPTY"] {
        let var = worker_env.iter().find(|var| var.name == name).unwrap();
        assert!(var.value.is_none());
        assert_eq!(
            var.value_from
                .as_ref()
                .unwrap()
                .secret_key_ref
                .as_ref()
                .unwrap()
                .name,
            "storm-deploy-1-env"
        );
    }
    // The init container that restores storage is told where to fetch through its
    // environment, and the URL is not part of its script
    let unpark = &pod.init_containers.as_ref().unwrap()[0];
    assert!(unpark.args.as_ref().unwrap()[1].contains("$STORMCHASER_FETCH_URL"));
    assert_eq!(
        unpark.env.as_ref().unwrap()[0].name,
        "STORMCHASER_FETCH_URL"
    );

    // No pod may exist before the Secret does
    assert_eq!(job.spec.as_ref().unwrap().suspend, Some(true));
}

#[test]
fn a_step_with_nothing_to_hide_needs_no_secret_and_starts_at_once() {
    let metadata = k8s_job_metadata(serde_json::json!({
        "image": "alpine:latest",
        "command": ["true"]
    }));
    let (job, secrets) = do_build_job_spec("storm-plain-1", &metadata, None, None).unwrap();
    assert!(secrets.is_empty());
    assert_eq!(job.spec.unwrap().suspend, None);
}

#[test]
fn the_step_secret_is_owned_by_its_job() {
    let metadata = k8s_job_metadata(serde_json::json!({
        "image": "alpine:latest",
        "env": [{"name": "TOKEN", "value": "t-1"}]
    }));
    let (mut job, secrets) = do_build_job_spec("storm-own-1", &metadata, None, None).unwrap();
    job.metadata.uid = Some("uid-123".into());

    let secret = secrets.into_secret("storm-own-1", &job);
    assert_eq!(secret.metadata.name.as_deref(), Some("storm-own-1-env"));
    let owner = &secret.metadata.owner_references.as_ref().unwrap()[0];
    assert_eq!(owner.kind, "Job");
    assert_eq!(owner.uid, "uid-123");
    assert_eq!(
        secret.string_data.unwrap().values().collect::<Vec<_>>(),
        vec!["t-1"]
    );
}

fn worker_resources(
    job: &k8s_openapi::api::batch::v1::Job,
) -> k8s_openapi::api::core::v1::ResourceRequirements {
    job.spec
        .as_ref()
        .unwrap()
        .template
        .spec
        .as_ref()
        .unwrap()
        .containers[0]
        .resources
        .clone()
        .unwrap()
}

#[test]
fn a_step_that_asks_for_nothing_is_still_bounded() {
    let metadata = k8s_job_metadata(serde_json::json!({ "image": "alpine:latest" }));
    let (job, _secrets) = do_build_job_spec("limits-1", &metadata, None, None).unwrap();

    let resources = worker_resources(&job);
    let limits = resources.limits.unwrap();
    assert_eq!(limits["cpu"].0, "1");
    assert_eq!(limits["memory"].0, "2Gi");
    assert_eq!(resources.requests.unwrap()["memory"].0, "2Gi");
    // And it cannot run for ever
    assert_eq!(job.spec.unwrap().active_deadline_seconds, Some(24 * 3600));
}

#[test]
fn a_steps_own_request_and_timeout_are_kept_within_the_limits() {
    let mut metadata = k8s_job_metadata(serde_json::json!({
        "image": "alpine:latest",
        "resources": {
            "requests": { "cpu": "250m", "memory": "256Mi" },
            "limits": { "cpu": "2", "memory": "4Gi" }
        },
        "active_deadline_seconds": 600
    }));
    metadata.step_dsl.timeout = Some("30m".to_string());
    let (job, _secrets) = do_build_job_spec("limits-2", &metadata, None, None).unwrap();

    let resources = worker_resources(&job);
    assert_eq!(resources.limits.as_ref().unwrap()["cpu"].0, "2");
    assert_eq!(resources.limits.as_ref().unwrap()["memory"].0, "4Gi");
    assert_eq!(resources.requests.as_ref().unwrap()["cpu"].0, "250m");
    assert_eq!(resources.requests.as_ref().unwrap()["memory"].0, "256Mi");
    // The shorter of the step's timeout and the deadline it set on the Job
    assert_eq!(job.spec.unwrap().active_deadline_seconds, Some(600));

    metadata.step_dsl.spec["active_deadline_seconds"] = serde_json::json!(86400);
    let (job, _secrets) = do_build_job_spec("limits-2", &metadata, None, None).unwrap();
    assert_eq!(job.spec.unwrap().active_deadline_seconds, Some(1800));
}

#[test]
fn a_step_that_asks_for_more_than_the_runner_allows_is_refused() {
    for (spec, timeout, expected) in [
        (
            serde_json::json!({ "image": "a", "resources": { "limits": { "memory": "64Gi" } } }),
            None,
            "memory",
        ),
        (
            serde_json::json!({ "image": "a", "resources": { "requests": { "cpu": "64" } } }),
            None,
            "CPU",
        ),
        (
            serde_json::json!({ "image": "a", "parallelism": 500 }),
            None,
            "parallelism",
        ),
        (
            serde_json::json!({ "image": "a", "completions": 500 }),
            None,
            "completions",
        ),
        (serde_json::json!({ "image": "a" }), Some("30d"), "timeout"),
    ] {
        let mut metadata = k8s_job_metadata(spec);
        metadata.step_dsl.timeout = timeout.map(str::to_string);
        let err = do_build_job_spec("limits-3", &metadata, None, None).unwrap_err();
        assert!(err.to_string().contains(expected), "{expected}: {err}");
    }
}
