use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, warn};

/// Extracts all leaf values from a JSON value to populate the redaction registry.
///
/// Uses a [`HashSet`] to prevent duplicate entries, which avoids inflating storage
/// and redundant work in any downstream redaction logic.
pub fn extract_sensitive_values(val: &Value, registry: &mut HashSet<String>) {
    match val {
        Value::String(s) if !s.is_empty() && s.len() > 3 => {
            // Avoid redacting tiny strings which could mask everything
            registry.insert(s.clone());
        }
        Value::Array(arr) => {
            for v in arr {
                extract_sensitive_values(v, registry);
            }
        }
        Value::Object(obj) => {
            for v in obj.values() {
                extract_sensitive_values(v, registry);
            }
        }
        _ => {}
    }
}

/// Decrypts SOPS secrets found in the repository.
pub async fn decrypt_sops_secrets(
    repo_dir: &Path,
    sops_file: Option<&str>,
    #[allow(unused_variables)] role_arn: Option<&str>,
) -> Result<(Value, Vec<String>)> {
    let file_name = sops_file.unwrap_or(".sops.yaml");

    // Check common sops names if no specific file was requested
    let possible_files = if sops_file.is_some() {
        vec![file_name.to_string()]
    } else {
        vec![
            "secrets.sops.yaml".to_string(),
            "secrets.sops.json".to_string(),
            ".sops.yaml".to_string(),
            ".sops.json".to_string(),
        ]
    };

    let mut target_path = None;
    for f in possible_files {
        let p = repo_dir.join(&f);
        if p.exists() {
            target_path = Some(p);
            break;
        }
    }

    let Some(target) = target_path else {
        debug!("No SOPS file found in repo, returning empty secrets");
        return Ok((serde_json::json!({}), vec![]));
    };

    debug!("Found SOPS file: {:?}", target);

    let mut envs = HashMap::new();

    #[cfg(feature = "aws-sops")]
    {
        if let Some(arn) = role_arn {
            debug!("Assuming AWS STS role {} for SOPS decryption", arn);
            let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            let sts = aws_sdk_sts::Client::new(&config);

            match sts
                .assume_role()
                .role_arn(arn)
                .role_session_name("stormchaser-sops")
                .send()
                .await
            {
                Ok(resp) => {
                    if let Some(creds) = resp.credentials() {
                        envs.insert(
                            "AWS_ACCESS_KEY_ID".to_string(),
                            creds.access_key_id().to_string(),
                        );
                        envs.insert(
                            "AWS_SECRET_ACCESS_KEY".to_string(),
                            creds.secret_access_key().to_string(),
                        );
                        envs.insert(
                            "AWS_SESSION_TOKEN".to_string(),
                            creds.session_token().to_string(),
                        );
                    }
                }
                Err(e) => {
                    warn!("Failed to assume AWS role {} for SOPS: {:?}", arn, e);
                    return Err(anyhow::anyhow!(
                        "Failed to assume AWS role for SOPS: {:?}",
                        e
                    ));
                }
            }
        }
    }

    let output = Command::new("sops")
        .arg("-d")
        .arg(&target)
        .envs(envs)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to execute sops binary. Ensure sops is installed and in PATH.")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("sops decryption failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse as JSON or YAML
    let secrets_val: Value = serde_yaml::from_str(&stdout)
        .or_else(|_| serde_json::from_str(&stdout))
        .context("Failed to parse decrypted SOPS output as JSON or YAML")?;

    let mut sensitive_set = HashSet::new();
    extract_sensitive_values(&secrets_val, &mut sensitive_set);
    let sensitive_values: Vec<String> = sensitive_set.into_iter().collect();

    Ok((secrets_val, sensitive_values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_sensitive_values() {
        let val = serde_json::json!({
            "key1": "secret1234",
            "key2": "too", // Too short to extract
            "arr": [
                "secret5678",
                {"nested": "secret9012"}
            ]
        });

        let mut registry = HashSet::new();
        extract_sensitive_values(&val, &mut registry);

        assert_eq!(registry.len(), 3);
        assert!(registry.contains("secret1234"));
        assert!(registry.contains("secret5678"));
        assert!(registry.contains("secret9012"));
        assert!(!registry.contains("too"));
    }
}
