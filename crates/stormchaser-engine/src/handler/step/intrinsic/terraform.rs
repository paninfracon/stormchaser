use serde_json::Value;
use stormchaser_model::dsl::CommonContainerSpec;

#[cfg(feature = "aws-sdk-sts")]
async fn assume_aws_role(
    run_id: uuid::Uuid,
    region: Option<&str>,
    assume_role_arn: &str,
    role_session_name: Option<&str>,
) -> anyhow::Result<Vec<stormchaser_model::dsl::EnvVar>> {
    let mut config_loader = aws_config::defaults(aws_config::BehaviorVersion::v2026_01_12());
    if let Some(r) = region {
        config_loader = config_loader.region(aws_config::Region::new(r.to_string()));
    }
    let config = config_loader.load().await;
    let sts_client = aws_sdk_sts::Client::new(&config);

    let session_name = role_session_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("stormchaser-tf-{}", run_id));

    let assume_role_res = sts_client
        .assume_role()
        .role_arn(assume_role_arn)
        .role_session_name(session_name)
        .send()
        .await?;

    if let Some(credentials) = assume_role_res.credentials() {
        let envs = vec![
            stormchaser_model::dsl::EnvVar {
                name: "AWS_ACCESS_KEY_ID".to_string(),
                value: credentials.access_key_id().to_string(),
            },
            stormchaser_model::dsl::EnvVar {
                name: "AWS_SECRET_ACCESS_KEY".to_string(),
                value: credentials.secret_access_key().to_string(),
            },
            stormchaser_model::dsl::EnvVar {
                name: "AWS_SESSION_TOKEN".to_string(),
                value: credentials.session_token().to_string(),
            },
        ];
        Ok(envs)
    } else {
        Err(anyhow::anyhow!("Missing credentials from assume_role"))
    }
}

fn build_terraform_command(
    workspace_dir: &str,
    backend_bucket: Option<&str>,
    backend_key: Option<&str>,
    region: Option<&str>,
    is_apply: bool,
    auto_approve: bool,
    out_file: &str,
) -> String {
    let mut init_cmd = format!("cd {} && terraform init", workspace_dir);
    if let Some(bucket) = backend_bucket {
        init_cmd.push_str(&format!(" -backend-config='bucket={}'", bucket));
    }
    if let Some(key) = backend_key {
        init_cmd.push_str(&format!(" -backend-config='key={}'", key));
    }
    if let Some(r) = region {
        init_cmd.push_str(&format!(" -backend-config='region={}'", r));
    }

    let mut run_cmd = if is_apply {
        format!(
            "terraform apply {} {}",
            if auto_approve { "-auto-approve" } else { "" },
            out_file
        )
    } else {
        format!("terraform plan -out={}", out_file)
    };

    if is_apply {
        // output raw JSON, flattening newlines with tr
        run_cmd.push_str(
            " && echo '' && echo -n '--- TF OUTPUTS --- ' && terraform output -json | tr -d '\\n'",
        );
    } else {
        // output a plan summary for log scraping, and also save the full plan text to plan.txt
        run_cmd.push_str(&format!(" && terraform show -no-color {} > plan.txt", out_file));
        run_cmd.push_str(&format!(
            " && echo '' && echo -n '--- TF PLAN SUMMARY --- ' && terraform show -no-color {} | grep -E '^Plan:|^No changes.' | tail -n 1",
            out_file
        ));
        run_cmd.push_str(&format!(
            " && echo '' && echo -n '--- TF PLAN JSON --- ' && terraform show -json {} | tr -d '\\n'",
            out_file
        ));
    }

    format!(
        "mkdir -p /tmp/.terraform_plugin_cache && {} && {}",
        init_cmd, run_cmd
    )
}

fn extract_destructive_change_count(plan_summary: &str) -> Option<u32> {
    if let Some(caps) = regex::Regex::new(r"(\d+) to destroy")
        .ok()
        .and_then(|re| re.captures(plan_summary))
    {
        if let Some(count_str) = caps.get(1) {
            if let Ok(count) = count_str.as_str().parse::<u32>() {
                if count > 0 {
                    return Some(count);
                }
            }
        }
    }
    None
}

pub async fn mutate_if_terraform(
    #[allow(unused_variables)] run_id: uuid::Uuid,
    step_type: &mut String,
    resolved_spec: &mut Value,
) -> anyhow::Result<()> {
    if step_type == "TerraformPlan" || step_type == "TerraformApply" {
        let is_apply = *step_type == "TerraformApply";

        let actual_spec = resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone();

        let workspace_dir = actual_spec
            .get("workspace_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let backend_bucket = actual_spec.get("backend_bucket").and_then(|v| v.as_str());
        let backend_key = actual_spec.get("backend_key").and_then(|v| v.as_str());
        let region = actual_spec.get("region").and_then(|v| v.as_str());
        let out_file = actual_spec
            .get("out_file")
            .and_then(|v| v.as_str())
            .unwrap_or("tfplan");
        let auto_approve = actual_spec
            .get("auto_approve")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        #[cfg(feature = "aws-sdk-sts")]
        let assume_role_arn = actual_spec
            .get("aws_assume_role_arn")
            .and_then(|v| v.as_str());
        #[cfg(feature = "aws-sdk-sts")]
        let role_session_name = actual_spec
            .get("aws_role_session_name")
            .and_then(|v| v.as_str());

        let script = build_terraform_command(
            workspace_dir,
            backend_bucket,
            backend_key,
            region,
            is_apply,
            auto_approve,
            out_file,
        );

        let mut envs = Vec::new();
        if let Some(r) = region {
            envs.push(stormchaser_model::dsl::EnvVar {
                name: "AWS_REGION".to_string(),
                value: r.to_string(),
            });
        }
        envs.push(stormchaser_model::dsl::EnvVar {
            name: "TF_PLUGIN_CACHE_DIR".to_string(),
            value: "/tmp/.terraform_plugin_cache".to_string(),
        });

        #[cfg(feature = "aws-sdk-sts")]
        if let Some(role_arn) = assume_role_arn {
            let mut sts_envs = assume_aws_role(run_id, region, role_arn, role_session_name).await?;
            envs.append(&mut sts_envs);
        }

        let storage_mounts: Option<Vec<stormchaser_model::dsl::StorageMount>> = actual_spec
            .get("storage_mounts")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let cpu = actual_spec
            .get("cpu")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let memory = actual_spec
            .get("memory")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let container_spec = CommonContainerSpec {
            image: "hashicorp/terraform:latest".to_string(),
            command: Some(vec!["sh".to_string(), "-c".to_string(), script]),
            args: None,
            env: if envs.is_empty() { None } else { Some(envs) },
            cpu,
            memory,
            privileged: None,
            storage_mounts,
        };

        *step_type = "RunContainer".to_string();
        if let Ok(val) = serde_json::to_value(container_spec) {
            *resolved_spec = val;
        }
    }
    Ok(())
}

pub fn mutate_if_terraform_approval(step_type: &mut String, resolved_spec: &mut Value) {
    if step_type == "TerraformApproval" {
        let actual_spec = resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone();

        let approvers = actual_spec.get("approvers").cloned();
        let plan_summary = actual_spec
            .get("plan_summary")
            .and_then(|v| v.as_str())
            .unwrap_or("Review Terraform Plan");

        let mut plan_description = format!("Terraform Plan Review: {}", plan_summary);
        let mut has_destroys = false;

        if let Some(count) = extract_destructive_change_count(plan_summary) {
            has_destroys = true;
            plan_description = format!(
                "🚨 WARNING: DESTRUCTIVE CHANGES ({} to destroy) 🚨\n\n{}",
                count, plan_description
            );
        }

        let input = stormchaser_model::dsl::Input {
            name: "approval_decision".to_string(),
            r#type: "string".to_string(),
            description: Some(plan_description),
            default: Some(serde_json::json!("Approve")),
            validation: None,
            options: Some(vec!["Approve".to_string(), "Reject".to_string()]),
            query: None,
        };

        let mut notify_spec: Option<stormchaser_model::dsl::EmailSpec> = actual_spec
            .get("notify")
            .cloned()
            .and_then(|n| serde_json::from_value(n).ok());

        if has_destroys {
            if let Some(notify) = notify_spec.as_mut() {
                let is_html = notify.html.unwrap_or(false);
                let banner = if is_html {
                    "<div style=\"background-color: #ffcccc; color: #cc0000; padding: 10px; border: 1px solid #cc0000; font-weight: bold; margin-bottom: 15px;\">🚨 WARNING: This Terraform plan contains destructive changes! 🚨</div>\n\n"
                } else {
                    "🚨 WARNING: This Terraform plan contains destructive changes! 🚨\n\n"
                };
                notify.body = format!("{}{}", banner, notify.body);
                if !notify.subject.starts_with("[WARNING]") {
                    notify.subject = format!("[WARNING] {}", notify.subject);
                }
            }
        }

        let approval_spec = stormchaser_model::dsl::ApprovalSpec {
            approvers: approvers.and_then(|a| serde_json::from_value(a).ok()),
            inputs: Some(vec![input]),
            notify: notify_spec,
            timeout: actual_spec
                .get("timeout")
                .and_then(|t| t.as_str().map(|s| s.to_string())),
        };

        *step_type = "Approval".to_string();
        if let Ok(val) = serde_json::to_value(approval_spec) {
            *resolved_spec = val;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        mutate_if_terraform(uuid::Uuid::new_v4(), &mut step_type, &mut spec)
            .await
            .unwrap();

        assert_eq!(step_type, "RunContainer");
        let image = spec.get("image").and_then(|v| v.as_str()).unwrap();
        assert_eq!(image, "hashicorp/terraform:latest");

        let command = spec
            .get("command")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        let script = command[2].as_str().unwrap();
        assert!(script.contains("terraform plan -out=tfplan"), "missing plan command: {}", script);
        assert!(script.contains("terraform show -no-color tfplan"), "missing show command: {}", script);
        assert!(script.contains("--- TF PLAN SUMMARY ---"), "missing plan summary marker: {}", script);
        assert!(script.contains("--- TF PLAN JSON ---"), "missing plan json marker: {}", script);
        // Should not contain apply commands
        assert!(!script.contains("terraform apply"), "unexpected apply command: {}", script);
    }

    #[tokio::test]
    async fn test_terraform_apply_mutates_to_run_container() {
        let mut step_type = "TerraformApply".to_string();
        let mut spec = serde_json::json!({
            "workspace_dir": "/tf",
            "out_file": "myplan"
        });

        mutate_if_terraform(uuid::Uuid::new_v4(), &mut step_type, &mut spec)
            .await
            .unwrap();

        assert_eq!(step_type, "RunContainer");
        let command = spec
            .get("command")
            .and_then(|v| v.as_array())
            .unwrap();
        let script = command[2].as_str().unwrap();
        assert!(script.contains("terraform apply"), "missing apply command: {}", script);
        assert!(script.contains("myplan"), "out_file not used in apply: {}", script);
        assert!(script.contains("--- TF OUTPUTS ---"), "missing tf outputs marker: {}", script);
        // Should not contain plan commands
        assert!(!script.contains("terraform plan"), "unexpected plan command: {}", script);
    }

    #[tokio::test]
    async fn test_terraform_plan_uses_out_file_consistently() {
        let mut step_type = "TerraformPlan".to_string();
        let mut spec = serde_json::json!({
            "out_file": "custom_plan_file"
        });

        mutate_if_terraform(uuid::Uuid::new_v4(), &mut step_type, &mut spec)
            .await
            .unwrap();

        let command = spec
            .get("command")
            .and_then(|v| v.as_array())
            .unwrap();
        let script = command[2].as_str().unwrap();
        // All terraform show invocations must use the custom out_file, not a hardcoded 'tfplan'
        assert!(script.contains("terraform plan -out=custom_plan_file"), "plan out_file: {}", script);
        assert!(script.contains("terraform show -no-color custom_plan_file"), "show uses out_file: {}", script);
        assert!(script.contains("terraform show -json custom_plan_file"), "show json uses out_file: {}", script);
        assert!(!script.contains("terraform show -no-color tfplan"), "hardcoded tfplan found: {}", script);
        assert!(!script.contains("terraform show -json tfplan"), "hardcoded tfplan in json: {}", script);
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

        mutate_if_terraform(uuid::Uuid::new_v4(), &mut step_type, &mut spec)
            .await
            .unwrap();

        assert_eq!(spec.get("cpu").and_then(|v| v.as_str()), Some("500m"));
        assert_eq!(spec.get("memory").and_then(|v| v.as_str()), Some("1Gi"));
        let mounts = spec.get("storage_mounts").and_then(|v| v.as_array()).unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].get("name").and_then(|v| v.as_str()), Some("tf-cache"));
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
        let inputs = spec.get("inputs").and_then(|v| v.as_array()).unwrap();
        let description = inputs[0]
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(description.contains("DESTRUCTIVE CHANGES"), "no warning in description: {}", description);
        assert!(description.contains("3 to destroy"), "destroy count missing: {}", description);
    }

    #[test]
    fn test_non_terraform_step_not_mutated() {
        let mut step_type = "RunContainer".to_string();
        let mut spec = serde_json::json!({ "image": "ubuntu:latest" });

        mutate_if_terraform_approval(&mut step_type, &mut spec);

        assert_eq!(step_type, "RunContainer");
        assert_eq!(spec.get("image").and_then(|v| v.as_str()), Some("ubuntu:latest"));
    }
}
