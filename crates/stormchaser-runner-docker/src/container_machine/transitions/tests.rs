use crate::container_machine::ContainerMetadata;
use crate::container_machine::*;
use bollard::Docker;
use std::collections::HashMap;
use stormchaser_model::dsl::Step;
use uuid::Uuid;

async fn get_docker() -> Docker {
    Docker::connect_with_local_defaults().unwrap()
}

fn create_test_metadata(image: &str, cmd: Vec<&str>) -> ContainerMetadata {
    let spec = serde_json::json!({
        "image": image,
        "command": cmd,
        "cpu_limit": null,
        "memory_limit": null,
        "env": null,
        "ports": null,
        "storage_mounts": null
    });

    ContainerMetadata {
        run_id: Uuid::new_v4(),
        step_id: Uuid::new_v4(),
        fencing_token: 0,
        step_dsl: Step {
            name: "test_step".to_string(),
            r#type: "RunContainer".to_string(),
            spec,
            condition: None,
            params: HashMap::new(),
            strategy: None,
            aggregation: vec![],
            iterate: None,
            iterate_as: None,
            steps: None,
            next: vec![],
            on_failure: None,
            aliases: std::collections::HashMap::new(),
            retry: None,
            timeout: None,
            allow_failure: None,
            start_marker: None,
            end_marker: None,
            outputs: vec![],
            reports: vec![],
            artifacts: None,
        },
        received_at: chrono::Utc::now(),
        encryption_key: None,
        storage: None,
        test_report_urls: None,
        registry_auth: None,
    }
}

#[tokio::test]
async fn test_container_lifecycle_success() {
    let docker = get_docker().await;
    let metadata = create_test_metadata("ubuntu:latest", vec!["echo", "hello"]);

    let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

    // Test Start
    let start_res = machine.start().await.expect("Failed to start container");
    let running_machine = match start_res {
        StartResult::Running(m) => m,
        StartResult::Failed(_) => panic!("Container failed to start"),
    };

    let container_name = running_machine.state.container_name.clone();

    // Test Wait
    let finished_machine = running_machine
        .wait()
        .await
        .expect("Failed to wait on container");
    let state = finished_machine.into_result();

    match state {
        ContainerState::Succeeded(metrics) => {
            assert_eq!(metrics.exit_code, Some(0));
        }
        ContainerState::Failed(msg, _) => panic!("Container failed: {}", msg),
    }

    // Ensure container is cleaned up
    let inspect = docker.inspect_container(&container_name, None).await;
    assert!(inspect.is_err(), "Container should be removed");
}

#[tokio::test]
async fn test_container_lifecycle_failure() {
    let docker = get_docker().await;
    let metadata = create_test_metadata("ubuntu:latest", vec!["false"]);

    let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

    let start_res = machine.start().await.expect("Failed to start container");
    let running_machine = match start_res {
        StartResult::Running(m) => m,
        StartResult::Failed(_) => panic!("Container failed to start"),
    };

    let container_name = running_machine.state.container_name.clone();

    let finished_machine = running_machine
        .wait()
        .await
        .expect("Failed to wait on container");
    let state = finished_machine.into_result();

    match state {
        ContainerState::Failed(msg, metrics) => {
            assert!(
                msg.contains("Some(1)"),
                "Unexpected failure message: {}",
                msg
            );
            assert_eq!(metrics.exit_code, Some(1));
        }
        ContainerState::Succeeded(_) => panic!("Container should have failed"),
    }

    // Ensure container is cleaned up
    let inspect = docker.inspect_container(&container_name, None).await;
    assert!(inspect.is_err(), "Container should be removed");
}

#[tokio::test]
async fn test_adopt_container() {
    let docker = get_docker().await;
    let metadata = create_test_metadata("ubuntu:latest", vec!["echo", "hello"]);
    let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

    let running_machine = machine.adopt("my-adopted-container".to_string());
    assert_eq!(running_machine.state.container_name, "my-adopted-container");
    assert!(running_machine.state.volumes_to_cleanup.is_empty());
}

#[tokio::test]
async fn test_clean_up_orphaned_container() {
    let docker = get_docker().await;
    let container_name = format!("test-cleanup-{}", Uuid::new_v4());

    // Create a real container to clean up
    use futures::StreamExt;
    let _ = docker
        .create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: "ubuntu:latest",
                ..Default::default()
            }),
            None,
            None,
        )
        .collect::<Vec<_>>()
        .await;

    let _ = docker
        .create_container(
            Some(bollard::container::CreateContainerOptions {
                name: container_name.as_str(),
                ..Default::default()
            }),
            bollard::container::Config {
                image: Some("ubuntu:latest".to_string()),
                cmd: Some(vec!["sleep".to_string(), "1000".to_string()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let metadata = create_test_metadata("ubuntu:latest", vec!["sleep", "1000"]);
    let machine = DockerContainerMachine::new(docker.clone(), metadata, None);

    machine.clean_up(&container_name).await.unwrap();

    // Verify it is gone
    let inspect = docker.inspect_container(&container_name, None).await;
    assert!(inspect.is_err(), "Container should be cleaned up");
}

#[tokio::test]
async fn test_into_result() {
    use super::super::{state, ContainerMetrics, ContainerState, DockerContainerMachine};
    let docker = get_docker().await;
    let metadata = create_test_metadata("ubuntu:latest", vec!["echo", "hello"]);
    let machine = DockerContainerMachine {
        nats: None,
        docker,
        metadata,
        state: state::Finished {
            result: ContainerState::Succeeded(ContainerMetrics::default()),
        },
    };
    let state = machine.into_result();
    match state {
        ContainerState::Succeeded(_) => {}
        _ => panic!("Expected Succeeded"),
    }
}
