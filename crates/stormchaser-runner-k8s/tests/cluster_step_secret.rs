//! Runs steps on a real cluster and checks that nothing secret can be read from the Job
//! or its pods, and that the step and its helpers still receive every value.
//!
//! Ignored by default. To run it against a disposable cluster:
//!
//! ```text
//! KUBECONFIG=... STORMCHASER_E2E_NAMESPACE=sc-steps \
//!   cargo test -p stormchaser-runner-k8s --test cluster_step_secret -- --ignored
//! ```
//!
//! The storage test also needs `STORMCHASER_AGENT_IMAGE` (an agent image the cluster can
//! pull, with `curl` in it) and `STORMCHASER_E2E_STORAGE_URL`, the base URL of a server
//! the pods can reach that answers `GET /seed...` with a gzipped tar holding
//! `hello.txt`, accepts any `PUT`, and lists the PUTs it received at `GET /puts`.

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, DeleteParams, ListParams, PropagationPolicy};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use stormchaser_runner_k8s::job_machine::{
    k8s_utils::step_secret_name, JobMetadata, JobState, K8sJobMachine, StartResult,
};

fn namespace() -> String {
    std::env::var("STORMCHASER_E2E_NAMESPACE").expect("STORMCHASER_E2E_NAMESPACE must be set")
}

fn metadata(name: &str, spec: serde_json::Value) -> JobMetadata {
    JobMetadata {
        run_id: uuid::Uuid::new_v4(),
        step_id: uuid::Uuid::new_v4(),
        fencing_token: 1,
        step_dsl: serde_json::from_value(json!({
            "name": name,
            "type": "RunContainer",
            "params": {},
            "spec": spec,
            "aggregation": [],
            "next": [],
            "outputs": [],
            "reports": []
        }))
        .unwrap(),
        namespace: namespace(),
        received_at: chrono::Utc::now(),
        cluster_version: "1.31".to_string(),
        encryption_key: None,
        storage: None,
        test_report_urls: None,
        registry_auth: None,
        runner_id: None,
    }
}

fn delete_in_background() -> DeleteParams {
    DeleteParams {
        propagation_policy: Some(PropagationPolicy::Background),
        ..Default::default()
    }
}

/// Everything about the step that someone allowed to read Jobs and pods, but not
/// Secrets, could see.
async fn visible_text(client: &kube::Client, job_name: &str) -> String {
    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace());
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace());
    let job = jobs.get(job_name).await.unwrap();
    let pod_list = pods
        .list(&ListParams::default().labels(&format!("job-name={job_name}")))
        .await
        .unwrap();
    assert!(!pod_list.items.is_empty(), "the Job has no pod yet");
    format!(
        "{}\n{}",
        serde_json::to_string(&job).unwrap(),
        serde_json::to_string(&pod_list.items).unwrap()
    )
}

async fn wait_for_pod(client: &kube::Client, job_name: &str) {
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace());
    for _ in 0..120 {
        let list = pods
            .list(&ListParams::default().labels(&format!("job-name={job_name}")))
            .await
            .unwrap();
        if !list.items.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("no pod appeared for {job_name}");
}

#[tokio::test]
#[ignore = "needs a Kubernetes cluster"]
async fn a_steps_environment_reaches_it_through_a_secret_and_shows_nowhere_else() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = kube::Client::try_default().await.unwrap();

    let metadata = metadata(
        "env-secret",
        json!({
            "image": "alpine:3",
            // Succeeds only if the values arrived intact, and stays up long enough to be
            // looked at. The command is part of the pod, so it must not spell the value.
            "command": ["sh", "-c",
                "[ \"${#DB_PASSWORD}\" = 28 ] && case \"$DB_PASSWORD\" in hunter*'\"chars\"') true ;; *) false ;; esac && test -z \"$EMPTY\" && test \"$PLAIN\" = ok && sleep 15"],
            "env": [
                { "name": "DB_PASSWORD", "value": "hunter2 with $pecial \"chars\"" },
                { "name": "EMPTY", "value": "" },
                { "name": "PLAIN", "value": "ok" }
            ]
        }),
    );
    let StartResult::Running(machine) = K8sJobMachine::new(client.clone(), metadata)
        .start()
        .await
        .unwrap()
    else {
        panic!("the Job did not start");
    };
    let job_name = machine.state.job_name.clone();
    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace());
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace());

    wait_for_pod(&client, &job_name).await;
    let visible = visible_text(&client, &job_name).await;
    assert!(
        !visible.contains("hunter2"),
        "the value is readable in the Job or its pod"
    );
    assert!(visible.contains("DB_PASSWORD"));

    // The Job was resumed, and its Secret belongs to it
    let job = jobs.get(&job_name).await.unwrap();
    assert_ne!(job.spec.as_ref().unwrap().suspend, Some(true));
    let secret = secrets.get(&step_secret_name(&job_name)).await.unwrap();
    let owner = &secret.metadata.owner_references.as_ref().unwrap()[0];
    assert_eq!(Some(&owner.uid), job.metadata.uid.as_ref());

    match machine.wait().await.unwrap() {
        JobState::Succeeded(_) => {}
        JobState::Failed(reason, _) => {
            panic!("the step did not receive its environment: {reason}")
        }
    }

    // Deleting the Job takes the Secret with it
    jobs.delete(&job_name, &delete_in_background())
        .await
        .unwrap();
    for _ in 0..120 {
        if secrets
            .get_opt(&step_secret_name(&job_name))
            .await
            .unwrap()
            .is_none()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the step's Secret outlived its Job");
}

#[tokio::test]
#[ignore = "needs a Kubernetes cluster, an agent image and a storage stand-in"]
async fn storage_is_restored_and_parked_without_its_urls_showing_in_the_pod() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (Ok(_agent), Ok(storage)) = (
        std::env::var("STORMCHASER_AGENT_IMAGE"),
        std::env::var("STORMCHASER_E2E_STORAGE_URL"),
    ) else {
        eprintln!("skipped: STORMCHASER_AGENT_IMAGE and STORMCHASER_E2E_STORAGE_URL are not set");
        return;
    };
    let client = kube::Client::try_default().await.unwrap();
    let marker = uuid::Uuid::new_v4().simple().to_string();

    let mut metadata = metadata(
        "storage-secret",
        json!({
            "image": "alpine:3",
            // The archive behind the fetch URL was unpacked into the volume
            "command": ["sh", "-c",
                "grep -q restored /workspace/hello.txt && echo built > /workspace/out.txt && sleep 10"],
            "storage_mounts": [{ "name": "workspace", "mount_path": "/workspace" }]
        }),
    );
    metadata.storage = Some(HashMap::from([(
        "workspace".to_string(),
        json!({
            "get_url": format!("{storage}/seed?sig=GET-{marker}"),
            "put_url": format!("{storage}/park?sig=PUT-{marker}"),
            "expected_hash": "previous-state"
        }),
    )]));

    let StartResult::Running(machine) = K8sJobMachine::new(client.clone(), metadata)
        .start()
        .await
        .unwrap()
    else {
        panic!("the Job did not start");
    };
    let job_name = machine.state.job_name.clone();
    wait_for_pod(&client, &job_name).await;
    let visible = visible_text(&client, &job_name).await;
    assert!(
        !visible.contains(&marker),
        "a presigned URL is readable in the Job or its pod"
    );

    let state = machine.wait().await.unwrap();
    let JobState::Succeeded(metrics) = state else {
        panic!("the step failed: {state:?}");
    };
    // The sidecar parked the volume, and reported its hash to the runner
    assert!(
        metrics
            .storage_hashes
            .as_ref()
            .is_some_and(|hashes| hashes.contains_key("workspace")),
        "no storage hash was reported: {metrics:?}"
    );
    // ...to the URL it was given through the Secret
    let puts = fetch_json(&format!("{storage}/puts")).await;
    assert!(
        puts.to_string().contains(&format!("PUT-{marker}")),
        "storage never received the upload: {puts}"
    );

    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace());
    jobs.delete(&job_name, &delete_in_background())
        .await
        .unwrap();
}

async fn fetch_json(url: &str) -> serde_json::Value {
    let output = tokio::process::Command::new("curl")
        .args(["-s", url])
        .output()
        .await
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}
