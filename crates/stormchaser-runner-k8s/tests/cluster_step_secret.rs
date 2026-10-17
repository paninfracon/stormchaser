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
            // The restore refuses an archive without the hash parking reported for it
            "expected_hash": sha256_of(&format!("{storage}/seed")).await
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

async fn sha256_of(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let output = tokio::process::Command::new("curl")
        .args(["-sf", url])
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "could not fetch {url}");
    Sha256::digest(&output.stdout)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn fetch_json(url: &str) -> serde_json::Value {
    let output = tokio::process::Command::new("curl")
        .args(["-s", url])
        .output()
        .await
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
#[ignore = "needs a Kubernetes cluster"]
async fn a_step_that_outlives_its_timeout_is_ended_by_the_cluster() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = kube::Client::try_default().await.unwrap();

    let mut metadata = metadata(
        "outlives-timeout",
        json!({ "image": "alpine:3", "command": ["sleep", "300"] }),
    );
    metadata.step_dsl.timeout = Some("5s".to_string());

    let started = std::time::Instant::now();
    let StartResult::Running(machine) = K8sJobMachine::new(client.clone(), metadata)
        .start()
        .await
        .unwrap()
    else {
        panic!("the Job did not start");
    };
    let job_name = machine.state.job_name.clone();

    // The limits the runner applied are on the Job, and the cluster accepted them
    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace());
    let job = jobs.get(&job_name).await.unwrap();
    let spec = job.spec.as_ref().unwrap();
    assert_eq!(spec.active_deadline_seconds, Some(5));
    let limits = spec.template.spec.as_ref().unwrap().containers[0]
        .resources
        .as_ref()
        .unwrap()
        .limits
        .as_ref()
        .unwrap();
    assert_eq!(limits["memory"].0, "2Gi");
    assert_eq!(limits["cpu"].0, "1");

    match machine.wait().await.unwrap() {
        JobState::Failed(reason, _) => {
            assert!(
                reason.to_lowercase().contains("deadline"),
                "the failure does not say why: {reason}"
            )
        }
        JobState::Succeeded(_) => panic!("a step that timed out was reported as a success"),
    }
    assert!(started.elapsed() < Duration::from_secs(120));

    jobs.delete(&job_name, &delete_in_background())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a Kubernetes cluster"]
async fn cancelling_a_run_deletes_its_jobs_and_no_others() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = kube::Client::try_default().await.unwrap();
    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace());
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace());

    let long_step = |name: &str| {
        metadata(
            name,
            json!({
                "image": "alpine:3",
                "command": ["sleep", "300"],
                // A step cannot take itself out of reach by naming its own run id
                "labels": { "stormchaser-run-id": "someone-elses" },
                "env": [{ "name": "TOKEN", "value": "s3cret" }]
            }),
        )
    };
    let cancelled = long_step("cancelled");
    let cancelled_run = cancelled.run_id;
    let bystander = long_step("bystander");
    let bystander_run = bystander.run_id;

    let mut machines = Vec::new();
    for metadata in [cancelled, bystander] {
        let StartResult::Running(machine) = K8sJobMachine::new(client.clone(), metadata)
            .start()
            .await
            .unwrap()
        else {
            panic!("the Job did not start");
        };
        wait_for_pod(&client, &machine.state.job_name).await;
        machines.push(machine);
    }
    let bystander_machine = machines.pop().unwrap();
    let cancelled_machine = machines.pop().unwrap();
    let cancelled_job = cancelled_machine.state.job_name.clone();
    let bystander_job = bystander_machine.state.job_name.clone();

    let started = std::time::Instant::now();
    let wait = tokio::spawn(cancelled_machine.wait());
    assert_eq!(
        stormchaser_runner_k8s::job_machine::k8s_utils::stop_run_jobs(
            &client,
            &namespace(),
            cancelled_run
        )
        .await,
        1
    );

    // The step's own handler notices and reports a failure, far sooner than the five
    // minutes the command asked for
    let outcome = tokio::time::timeout(Duration::from_secs(120), wait)
        .await
        .expect("the cancelled step's handler never returned")
        .unwrap();
    if let Ok(JobState::Succeeded(_)) = outcome {
        panic!("a cancelled step was reported as a success");
    }
    assert!(started.elapsed() < Duration::from_secs(120));

    // Its Job, pods and Secret go
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace());
    let mut gone = false;
    for _ in 0..180 {
        let job = jobs.get_opt(&cancelled_job).await.unwrap();
        let pod_count = pods
            .list(&ListParams::default().labels(&format!("job-name={cancelled_job}")))
            .await
            .unwrap()
            .items
            .len();
        let secret = secrets
            .get_opt(&step_secret_name(&cancelled_job))
            .await
            .unwrap();
        if job.is_none() && pod_count == 0 && secret.is_none() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(gone, "the cancelled run left a Job, pod or Secret behind");

    // The other run's Job is untouched
    assert!(jobs.get_opt(&bystander_job).await.unwrap().is_some());
    assert_eq!(
        stormchaser_runner_k8s::job_machine::k8s_utils::stop_run_jobs(
            &client,
            &namespace(),
            bystander_run
        )
        .await,
        1
    );
    drop(bystander_machine);
}
