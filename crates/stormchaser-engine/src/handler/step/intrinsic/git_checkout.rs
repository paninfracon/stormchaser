use anyhow::Context;
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::dsl;

/// Mutates the git_checkout step spec to automatically configure parameters like repository, reference, and token based on context if they are omitted.
pub async fn mutate(
    step_type: &mut String,
    resolved_spec: &mut Value,
    pool: Option<&PgPool>,
) -> anyhow::Result<()> {
    if step_type == "GitCheckout" {
        let git_spec: Result<dsl::GitCheckoutSpec, _> =
            serde_json::from_value(resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone());

        if let Ok(git) = git_spec {
            let depth = git.depth.unwrap_or(1);
            let dest = git.destination.clone().unwrap_or_else(|| ".".to_string());
            let branch = git.r#ref.clone().unwrap_or_else(|| "main".to_string());

            let mut auth_command = String::new();
            let mut env_vars = Vec::new();

            if let Some(conn_name) = &git.connection {
                let pool = pool.with_context(|| {
                    format!(
                        "GitCheckout step references connection '{}' but no database context is available",
                        conn_name
                    )
                })?;
                let conn = crate::db::connections::get_storage_backend_by_name::<
                    _,
                    stormchaser_model::Connection,
                >(pool, conn_name)
                .await?
                .with_context(|| format!("GitCheckout connection '{}' was not found", conn_name))?;

                if conn.connection_type != stormchaser_model::connections::ConnectionType::Git {
                    anyhow::bail!(
                        "GitCheckout connection '{}' has type {:?}, expected git",
                        conn_name,
                        conn.connection_type
                    );
                }

                if let Some(creds) = &conn.encrypted_credentials {
                    if let Some(username) = conn.config.get("username").and_then(|v| v.as_str()) {
                        env_vars.push(dsl::EnvVar {
                            name: "GIT_USERNAME".to_string(),
                            value: username.to_string(),
                        });
                        env_vars.push(dsl::EnvVar {
                            name: "GIT_PASSWORD".to_string(),
                            value: creds.to_string(),
                        });
                        auth_command = "git config --global credential.helper '!f() { echo username=$GIT_USERNAME; echo password=$GIT_PASSWORD; }; f' && ".to_string();
                    } else {
                        env_vars.push(dsl::EnvVar {
                            name: "GIT_BEARER_TOKEN".to_string(),
                            value: creds.to_string(),
                        });
                        auth_command = "git config --global http.extraHeader \"Authorization: Bearer $GIT_BEARER_TOKEN\" && ".to_string();
                    }
                }

                if let Some(ssh_key) = conn.config.get("ssh_key").and_then(|v| v.as_str()) {
                    env_vars.push(dsl::EnvVar {
                        name: "GIT_SSH_KEY".to_string(),
                        value: ssh_key.to_string(),
                    });
                    auth_command = "mkdir -p ~/.ssh && echo \"$GIT_SSH_KEY\" > ~/.ssh/id_rsa && chmod 600 ~/.ssh/id_rsa && export GIT_SSH_COMMAND='ssh -i ~/.ssh/id_rsa -o StrictHostKeyChecking=no' && ".to_string();
                }
            }

            let mut script = format!(
                "{}mkdir -p {dest} && cd {dest} && git init && git remote add origin {repo} && git fetch --depth {depth} --filter=blob:none origin {branch} && git checkout FETCH_HEAD",
                auth_command,
                dest = dest,
                repo = git.repo,
                depth = depth,
                branch = branch
            );

            if let Some(sparse) = git.sparse_checkout {
                if !sparse.is_empty() {
                    let sparse_paths = sparse.join(" ");
                    script = format!(
                        "{}mkdir -p {dest} && cd {dest} && git init && git remote add origin {repo} && git sparse-checkout set {sparse_paths} && git fetch --depth {depth} --filter=blob:none origin {branch} && git checkout FETCH_HEAD",
                        auth_command,
                        dest = dest,
                        repo = git.repo,
                        sparse_paths = sparse_paths,
                        depth = depth,
                        branch = branch
                    );
                }
            }

            let container_spec = dsl::CommonContainerSpec {
                image: "alpine/git:latest".to_string(),
                registry_connection: None,
                connections: None,
                command: Some(vec!["sh".to_string(), "-c".to_string(), script]),
                args: None,
                env: if env_vars.is_empty() {
                    None
                } else {
                    Some(env_vars)
                },
                cpu: None,
                memory: None,
                privileged: None,
                storage_mounts: git.storage_mounts,
            };

            *step_type = "RunContainer".to_string();
            if let Ok(val) = serde_json::to_value(container_spec) {
                *resolved_spec = val;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mutate_git_checkout_default() {
        let mut step_type = "GitCheckout".to_string();
        let mut spec = serde_json::json!({
            "repo": "https://github.com/example/repo.git"
        });

        mutate(&mut step_type, &mut spec, None).await.unwrap();

        assert_eq!(step_type, "RunContainer");
        let spec_obj = spec.as_object().unwrap();
        assert_eq!(spec_obj.get("image").unwrap(), "alpine/git:latest");
        let command = spec_obj.get("command").unwrap().as_array().unwrap();
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert_eq!(
            command[2],
            "mkdir -p . && cd . && git init && git remote add origin https://github.com/example/repo.git && git fetch --depth 1 --filter=blob:none origin main && git checkout FETCH_HEAD"
        );
    }

    #[tokio::test]
    async fn test_mutate_git_checkout_sparse() {
        let mut step_type = "GitCheckout".to_string();
        let mut spec = serde_json::json!({
            "repo": "https://github.com/example/repo.git",
            "ref": "v1.0",
            "depth": 5,
            "sparse_checkout": ["src/", "docs/"],
            "destination": "/workspace/repo"
        });

        mutate(&mut step_type, &mut spec, None).await.unwrap();

        assert_eq!(step_type, "RunContainer");
        let spec_obj = spec.as_object().unwrap();
        let command = spec_obj.get("command").unwrap().as_array().unwrap();
        assert_eq!(
            command[2],
            "mkdir -p /workspace/repo && cd /workspace/repo && git init && git remote add origin https://github.com/example/repo.git && git sparse-checkout set src/ docs/ && git fetch --depth 5 --filter=blob:none origin v1.0 && git checkout FETCH_HEAD"
        );
    }
}
