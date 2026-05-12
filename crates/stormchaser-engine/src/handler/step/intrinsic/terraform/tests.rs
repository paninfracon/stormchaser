use super::*;
use uuid::Uuid;

#[tokio::test]
async fn test_terraform_plan_mutates_to_run_container() {
    let mut step_type = "TerraformPlan".to_string();
    let mut spec = serde_json::json!({
        "workspace_dir": "/tf",
        "backend_bucket": "my-bucket",
        "backend_key": "state/key",
        "region": "us-east-1",
        "out_file": "tfplan"
    });

    mutate_if_terraform(Uuid::new_v4(), &mut step_type, &mut spec)
        .await
        .unwrap();

    assert_eq!(step_type, "RunContainer");
    let image = spec.get("image").and_then(|v| v.as_str()).unwrap();
    assert_eq!(image, "hashicorp/terraform:latest");

    let command = spec.get("command").and_then(|v| v.as_array()).unwrap();
    assert_eq!(command[0], "sh");
    assert_eq!(command[1], "-c");
    let script = command[2].as_str().unwrap();
    assert!(
        script.contains("terraform plan -out=tfplan"),
        "missing plan command: {}",
        script
    );
    assert!(
        script.contains("terraform show -no-color tfplan"),
        "missing show command: {}",
        script
    );
    assert!(
        script.contains("--- TF PLAN SUMMARY ---"),
        "missing plan summary marker: {}",
        script
    );
    assert!(
        script.contains("--- TF PLAN JSON ---"),
        "missing plan json marker: {}",
        script
    );
    // Should not contain apply commands
    assert!(
        !script.contains("terraform apply"),
        "unexpected apply command: {}",
        script
    );
}

#[tokio::test]
async fn test_terraform_apply_mutates_to_run_container() {
    let mut step_type = "TerraformApply".to_string();
    let mut spec = serde_json::json!({
        "workspace_dir": "/tf",
        "out_file": "myplan"
    });

    mutate_if_terraform(Uuid::new_v4(), &mut step_type, &mut spec)
        .await
        .unwrap();

    assert_eq!(step_type, "RunContainer");
    let command = spec.get("command").and_then(|v| v.as_array()).unwrap();
    let script = command[2].as_str().unwrap();
    assert!(
        script.contains("terraform apply"),
        "missing apply command: {}",
        script
    );
    assert!(
        script.contains("myplan"),
        "out_file not used in apply: {}",
        script
    );
    assert!(
        script.contains("--- TF OUTPUTS ---"),
        "missing tf outputs marker: {}",
        script
    );
    // Should not contain plan commands
    assert!(
        !script.contains("terraform plan"),
        "unexpected plan command: {}",
        script
    );
}

#[tokio::test]
async fn test_terraform_plan_uses_out_file_consistently() {
    let mut step_type = "TerraformPlan".to_string();
    let mut spec = serde_json::json!({
        "out_file": "custom_plan_file"
    });

    mutate_if_terraform(Uuid::new_v4(), &mut step_type, &mut spec)
        .await
        .unwrap();

    let command = spec.get("command").and_then(|v| v.as_array()).unwrap();
    let script = command[2].as_str().unwrap();
    // All terraform show invocations must use the custom out_file, not a hardcoded 'tfplan'
    assert!(
        script.contains("terraform plan -out=custom_plan_file"),
        "plan out_file: {}",
        script
    );
    assert!(
        script.contains("terraform show -no-color custom_plan_file"),
        "show uses out_file: {}",
        script
    );
    assert!(
        script.contains("terraform show -json custom_plan_file"),
        "show json uses out_file: {}",
        script
    );
    assert!(
        !script.contains("terraform show -no-color tfplan"),
        "hardcoded tfplan found: {}",
        script
    );
    assert!(
        !script.contains("terraform show -json tfplan"),
        "hardcoded tfplan in json: {}",
        script
    );
}

#[tokio::test]
async fn test_terraform_plan_passthrough_storage_mounts_and_resources() {
    let mut step_type = "TerraformPlan".to_string();
    let mut spec = serde_json::json!({
        "cpu": "500m",
        "memory": "1Gi",
        "storage_mounts": [
            { "name": "tf-cache", "mount_path": "/tf-cache", "read_only": false }
        ]
    });

    mutate_if_terraform(Uuid::new_v4(), &mut step_type, &mut spec)
        .await
        .unwrap();

    assert_eq!(spec.get("cpu").and_then(|v| v.as_str()), Some("500m"));
    assert_eq!(spec.get("memory").and_then(|v| v.as_str()), Some("1Gi"));
    let mounts = spec
        .get("storage_mounts")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(mounts.len(), 1);
    assert_eq!(
        mounts[0].get("name").and_then(|v| v.as_str()),
        Some("tf-cache")
    );
}

#[test]
fn test_terraform_approval_mutates_to_approval() {
    let mut step_type = "TerraformApproval".to_string();
    let mut spec = serde_json::json!({
        "approvers": ["admin", "ops"],
        "plan_summary": "Plan: 2 to add, 0 to change, 0 to destroy.",
        "timeout": "24h"
    });

    mutate_if_terraform_approval(&mut step_type, &mut spec);

    assert_eq!(step_type, "Approval");
    let approvers = spec.get("approvers").and_then(|v| v.as_array()).unwrap();
    assert_eq!(approvers.len(), 2);
    assert_eq!(approvers[0].as_str(), Some("admin"));
    assert_eq!(approvers[1].as_str(), Some("ops"));
    assert_eq!(spec.get("timeout").and_then(|v| v.as_str()), Some("24h"));
}

#[test]
fn test_terraform_approval_destructive_changes_adds_warning() {
    let mut step_type = "TerraformApproval".to_string();
    let mut spec = serde_json::json!({
        "approvers": ["admin"],
        "plan_summary": "Plan: 1 to add, 0 to change, 3 to destroy."
    });

    mutate_if_terraform_approval(&mut step_type, &mut spec);

    assert_eq!(step_type, "Approval");
    // The approval input description should contain the WARNING banner
    let inputs = spec.get("inputs").and_then(|v| v.as_object()).unwrap();
    let description = inputs
        .get("properties")
        .and_then(|p| p.get("approval_decision"))
        .and_then(|a| a.get("description"))
        .and_then(|d| d.as_str())
        .unwrap();
    assert!(
        description.contains("DESTRUCTIVE CHANGES"),
        "no warning in description: {}",
        description
    );
    assert!(
        description.contains("3 to destroy"),
        "destroy count missing: {}",
        description
    );
}

#[test]
fn test_non_terraform_step_not_mutated() {
    let mut step_type = "RunContainer".to_string();
    let mut spec = serde_json::json!({ "image": "ubuntu:latest" });

    mutate_if_terraform_approval(&mut step_type, &mut spec);

    assert_eq!(step_type, "RunContainer");
    assert_eq!(
        spec.get("image").and_then(|v| v.as_str()),
        Some("ubuntu:latest")
    );
}
