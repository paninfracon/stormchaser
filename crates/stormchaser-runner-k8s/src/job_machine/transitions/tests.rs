use super::*;

#[test]
fn test_do_parse_version() {
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("1.2.3").unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("v1.2.3").unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("v1.2.3-alpha").unwrap(),
        vec![1, 2, 3]
    );
    assert_eq!(
        K8sJobMachine::<state::Initialized>::do_parse_version("1.25.0+xyz").unwrap(),
        vec![1, 25, 0]
    );
    K8sJobMachine::<state::Initialized>::do_parse_version("invalid").unwrap_err();
}

#[test]
fn test_do_check_version() {
    K8sJobMachine::<state::Initialized>::do_check_version("1.25.0", "1.24.0").unwrap();
    K8sJobMachine::<state::Initialized>::do_check_version("1.25.0", "1.25.0").unwrap();
    K8sJobMachine::<state::Initialized>::do_check_version("1.25.1", "1.25.0").unwrap();

    let err =
        K8sJobMachine::<state::Initialized>::do_check_version("1.24.0", "1.25.0").unwrap_err();
    assert!(err.to_string().contains("less than required version"));
}

#[test]
fn pull_secret_is_owned_by_its_job_and_keyed_by_the_registry() {
    use k8s_openapi::api::batch::v1::Job;

    let mut job = Job::default();
    job.metadata.uid = Some("0b1c2d3e-uid".to_string());
    let auth = serde_json::json!({
        "username": "robot",
        "password": "s3cret",
        "url": "registry.example.com",
    });

    let secret = super::build_pull_secret("storm-build-1234abcd", &auth, &job);

    assert_eq!(
        secret.metadata.name.as_deref(),
        Some("storm-build-1234abcd-auth")
    );
    let owners = secret.metadata.owner_references.expect("an owner");
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].kind, "Job");
    assert_eq!(owners[0].name, "storm-build-1234abcd");
    assert_eq!(owners[0].uid, "0b1c2d3e-uid");

    let config: serde_json::Value =
        serde_json::from_slice(&secret.data.unwrap()[".dockerconfigjson"].0).unwrap();
    let auths = config["auths"].as_object().unwrap();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths["registry.example.com"]["username"], "robot");
}

#[test]
fn pull_secret_without_a_registry_url_is_for_docker_hub() {
    use k8s_openapi::api::batch::v1::Job;

    let auth = serde_json::json!({"username": "robot", "password": "s3cret", "url": ""});
    let secret = super::build_pull_secret("storm-x-1", &auth, &Job::default());
    let config: serde_json::Value =
        serde_json::from_slice(&secret.data.unwrap()[".dockerconfigjson"].0).unwrap();
    assert!(config["auths"]
        .as_object()
        .unwrap()
        .contains_key("https://index.docker.io/v1/"));
}
