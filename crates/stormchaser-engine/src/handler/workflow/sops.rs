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

    // `sops_file` comes from the run request. It must stay inside this repository and may
    // not be, or pass through, a symlink; a named file that fails that check is an error
    // rather than "no secrets".
    let mut target_path = None;
    for f in possible_files {
        match crate::git_cache::safe_worktree_path(repo_dir, &f) {
            Ok(p) => {
                target_path = Some(p);
                break;
            }
            Err(e) => {
                let missing = crate::git_cache::validate_relative_path(&f).is_ok()
                    && repo_dir.join(&f).symlink_metadata().is_err();
                if sops_file.is_some() && !missing {
                    return Err(e.context("Refusing SOPS file"));
                }
            }
        }
    }

    let Some(target) = target_path else {
        debug!("No SOPS file found in repo, returning empty secrets");
        return Ok((serde_json::json!({}), vec![]));
    };

    debug!("Found SOPS file: {:?}", target);

    // `.sops.yaml` is also the name of sops's own configuration file, which holds no
    // secrets. A file found by name alone is only decrypted when it carries SOPS metadata.
    if sops_file.is_none() {
        let content = tokio::fs::read_to_string(&target)
            .await
            .with_context(|| format!("Failed to read {:?}", target))?;
        if !has_sops_metadata(&content) {
            debug!("{:?} is not a SOPS-encrypted file; no secrets", target);
            return Ok((serde_json::json!({}), vec![]));
        }
    }

    let mut envs = HashMap::new();

    #[cfg(feature = "aws-sops")]
    {
        if let Some(arn) = role_arn {
            crate::aws_guard::ensure_role_allowed(arn)?;
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

    let secrets_val = parse_decrypted_output(&stdout)?;

    let mut sensitive_set = HashSet::new();
    extract_sensitive_values(&secrets_val, &mut sensitive_set);
    let sensitive_values: Vec<String> = sensitive_set.into_iter().collect();

    Ok((secrets_val, sensitive_values))
}

/// Whether a YAML or JSON document is SOPS-encrypted, which sops marks with a top-level
/// `sops` metadata key.
fn has_sops_metadata(content: &str) -> bool {
    parse_decrypted_output(content)
        .map(|doc| doc.get("sops").is_some_and(Value::is_object))
        .unwrap_or(false)
}

/// Parses decrypted SOPS output, which may be YAML or JSON.
fn parse_decrypted_output(output: &str) -> Result<Value> {
    serde_saphyr::from_str(output)
        .or_else(|_| serde_json::from_str(output))
        .context("Failed to parse decrypted SOPS output as JSON or YAML")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_decrypted_output_yaml() {
        let yaml = "db:\n  password: s3cret\n  port: 5432\n  tls: true\ntokens:\n  - abc\n  - \"123\"\nempty: null\n";
        let parsed = parse_decrypted_output(yaml).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({
                "db": { "password": "s3cret", "port": 5432, "tls": true },
                "tokens": ["abc", "123"],
                "empty": null
            })
        );
    }

    #[test]
    fn test_parse_decrypted_output_json() {
        let json = r#"{"api_key": "abc", "nested": {"n": 1.5, "list": [1, 2]}}"#;
        let parsed = parse_decrypted_output(json).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!({"api_key": "abc", "nested": {"n": 1.5, "list": [1, 2]}})
        );
    }

    #[test]
    fn test_parse_decrypted_output_rejects_garbage() {
        parse_decrypted_output("key: [unterminated").unwrap_err();
    }

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

    #[test]
    fn test_has_sops_metadata() {
        let config = "creation_rules:\n  - age: age1paninfracon\n";
        assert!(!has_sops_metadata(config));

        let yaml = "db_password: ENC[AES256_GCM,data:abc,type:str]\nsops:\n  version: 3.9.0\n";
        assert!(has_sops_metadata(yaml));

        let json =
            r#"{"token": "ENC[AES256_GCM,data:abc,type:str]", "sops": {"version": "3.9.0"}}"#;
        assert!(has_sops_metadata(json));

        assert!(!has_sops_metadata("sops: plain-string\n"));
        assert!(!has_sops_metadata("not: [valid"));
    }

    #[tokio::test]
    async fn test_sops_config_file_means_no_secrets() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(
            repo.path().join(".sops.yaml"),
            "creation_rules:\n  - age: age1paninfracon\n",
        )
        .unwrap();

        let (secrets, sensitive) = decrypt_sops_secrets(repo.path(), None, None).await.unwrap();
        assert_eq!(secrets, serde_json::json!({}));
        assert!(sensitive.is_empty());
    }
}
