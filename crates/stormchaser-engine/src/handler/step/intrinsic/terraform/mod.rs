use serde_json::Value;
use stormchaser_model::dsl::ApprovalSpec;
use stormchaser_model::dsl::CommonContainerSpec;
use stormchaser_model::dsl::EmailSpec;
use stormchaser_model::dsl::EnvVar;
use stormchaser_model::dsl::StorageMount;

#[cfg(feature = "aws-sdk-sts")]
async fn assume_aws_role(
    run_id: uuid::Uuid,
    region: Option<&str>,
    assume_role_arn: &str,
    role_session_name: Option<&str>,
) -> anyhow::Result<Vec<EnvVar>> {
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
            EnvVar {
                name: "AWS_ACCESS_KEY_ID".to_string(),
                value: credentials.access_key_id().to_string(),
            },
            EnvVar {
                name: "AWS_SECRET_ACCESS_KEY".to_string(),
                value: credentials.secret_access_key().to_string(),
            },
            EnvVar {
                name: "AWS_SESSION_TOKEN".to_string(),
                value: credentials.session_token().to_string(),
            },
        ];
        Ok(envs)
    } else {
        Err(anyhow::anyhow!("Missing credentials from assume_role"))
    }
}

/// The image Terraform steps run in, pinned by digest so that what may hold an assumed
/// role's credentials does not change under a moving tag.
/// `STORMCHASER_TERRAFORM_IMAGE` replaces it.
const DEFAULT_TERRAFORM_IMAGE: &str = "hashicorp/terraform:1.16.6@sha256:bfffc1ded5fbf0a17beaaf22eb4793980abdb19d0761ff6a3a0efb2ab4d9b316";

fn terraform_image() -> String {
    std::env::var("STORMCHASER_TERRAFORM_IMAGE")
        .ok()
        .filter(|image| !image.is_empty())
        .unwrap_or_else(|| DEFAULT_TERRAFORM_IMAGE.to_string())
}

/// The step's script. The workspace, backend settings and plan file reach it as
/// environment variables (see [`terraform_env`]), never as part of its text: they can
/// come from a run's inputs, and the container may hold an assumed role's credentials.
fn build_terraform_command(
    has_backend_bucket: bool,
    has_backend_key: bool,
    has_region: bool,
    is_apply: bool,
    auto_approve: bool,
) -> String {
    let mut init_cmd = r#"cd -- "$STORMCHASER_TF_DIR" && terraform init"#.to_string();
    if has_backend_bucket {
        init_cmd.push_str(r#" -backend-config="bucket=$STORMCHASER_TF_BACKEND_BUCKET""#);
    }
    if has_backend_key {
        init_cmd.push_str(r#" -backend-config="key=$STORMCHASER_TF_BACKEND_KEY""#);
    }
    if has_region {
        init_cmd.push_str(r#" -backend-config="region=$AWS_REGION""#);
    }

    let mut run_cmd = if is_apply {
        format!(
            r#"terraform apply {}"$STORMCHASER_TF_OUT""#,
            if auto_approve { "-auto-approve " } else { "" },
        )
    } else {
        r#"terraform plan -out="$STORMCHASER_TF_OUT""#.to_string()
    };

    if is_apply {
        // output raw JSON, flattening newlines with tr
        run_cmd.push_str(
            " && echo '' && echo -n '--- TF OUTPUTS --- ' && terraform output -json | tr -d '\\n'",
        );
    } else {
        // output a plan summary for log scraping, and also save the full plan text to plan.txt
        run_cmd.push_str(r#" && terraform show -no-color "$STORMCHASER_TF_OUT" > plan.txt"#);
        run_cmd.push_str(
            r#" && echo '' && echo -n '--- TF PLAN SUMMARY --- ' && terraform show -no-color "$STORMCHASER_TF_OUT" | grep -E '^Plan:|^No changes.' | tail -n 1"#,
        );
        run_cmd.push_str(
            r#" && echo '' && echo -n '--- TF PLAN JSON --- ' && terraform show -json "$STORMCHASER_TF_OUT" | tr -d '\n'"#,
        );
    }

    format!(
        "mkdir -p /tmp/.terraform_plugin_cache && {} && {}",
        init_cmd, run_cmd
    )
}

/// The values [`build_terraform_command`] reads. The plan file is a positional argument
/// to `terraform apply`, so one that starts with `-` would be read as an option.
fn terraform_env(
    workspace_dir: &str,
    backend_bucket: Option<&str>,
    backend_key: Option<&str>,
    out_file: &str,
) -> anyhow::Result<Vec<EnvVar>> {
    if out_file.is_empty() || out_file.starts_with('-') {
        anyhow::bail!("Terraform out_file {:?} is not a file name", out_file);
    }
    let mut envs = vec![
        EnvVar {
            name: "STORMCHASER_TF_DIR".to_string(),
            value: workspace_dir.to_string(),
        },
        EnvVar {
            name: "STORMCHASER_TF_OUT".to_string(),
            value: out_file.to_string(),
        },
    ];
    for (name, value) in [
        ("STORMCHASER_TF_BACKEND_BUCKET", backend_bucket),
        ("STORMCHASER_TF_BACKEND_KEY", backend_key),
    ] {
        if let Some(value) = value {
            envs.push(EnvVar {
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }
    Ok(envs)
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
        mutate_terraform_spec(run_id, step_type, resolved_spec).await?;
    }
    Ok(())
}

async fn mutate_terraform_spec(
    run_id: uuid::Uuid,
    step_type: &mut String,
    resolved_spec: &mut Value,
) -> anyhow::Result<()> {
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
        backend_bucket.is_some(),
        backend_key.is_some(),
        region.is_some(),
        is_apply,
        auto_approve,
    );

    let mut envs = terraform_env(workspace_dir, backend_bucket, backend_key, out_file)?;
    if let Some(r) = region {
        envs.push(EnvVar {
            name: "AWS_REGION".to_string(),
            value: r.to_string(),
        });
    }
    envs.push(EnvVar {
        name: "TF_PLUGIN_CACHE_DIR".to_string(),
        value: "/tmp/.terraform_plugin_cache".to_string(),
    });

    #[cfg(feature = "aws-sdk-sts")]
    if let Some(role_arn) = assume_role_arn {
        crate::aws_guard::ensure_role_allowed(role_arn)?;
        let mut sts_envs = assume_aws_role(run_id, region, role_arn, role_session_name).await?;
        envs.append(&mut sts_envs);
    }

    let storage_mounts: Option<Vec<StorageMount>> = match actual_spec.get("storage_mounts") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(mounts) => Some(mounts),
            Err(err) => {
                tracing::warn!(
                    "Failed to deserialize storage_mounts for Terraform step, \
                         mounts will not be applied: {}",
                    err
                );
                None
            }
        },
        None => None,
    };
    let cpu = actual_spec
        .get("cpu")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let memory = actual_spec
        .get("memory")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let container_spec = CommonContainerSpec {
        registry_connection: None,
        connections: None,
        image: terraform_image(),
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

        let input_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "approval_decision": {
                    "type": "string",
                    "description": plan_description,
                    "default": "Approve",
                    "enum": ["Approve", "Reject"]
                }
            },
            "required": ["approval_decision"]
        });

        let mut notify_spec: Option<EmailSpec> = actual_spec
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

        let approval_spec = ApprovalSpec {
            approvers: approvers.and_then(|a| serde_json::from_value(a).ok()),
            inputs: Some(input_schema),
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
mod tests;
