use stormchaser_model::dsl;

pub fn mutate(step_type: &mut String, resolved_spec: &mut serde_json::Value) {
    if step_type == "GitCheckout" {
        let git_spec: Result<dsl::GitCheckoutSpec, _> =
            serde_json::from_value(resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone());

        if let Ok(git) = git_spec {
            let depth = git.depth.unwrap_or(1);
            let dest = git.destination.clone().unwrap_or_else(|| ".".to_string());
            let branch = git.r#ref.clone().unwrap_or_else(|| "main".to_string());

            let mut script = format!(
                "mkdir -p {dest} && cd {dest} && git init && git remote add origin {repo} && git fetch --depth {depth} --filter=blob:none origin {branch} && git checkout FETCH_HEAD",
                dest = dest,
                repo = git.repo,
                depth = depth,
                branch = branch
            );

            if let Some(sparse) = git.sparse_checkout {
                if !sparse.is_empty() {
                    let sparse_paths = sparse.join(" ");
                    script = format!(
                        "mkdir -p {dest} && cd {dest} && git init && git remote add origin {repo} && git sparse-checkout set {sparse_paths} && git fetch --depth {depth} --filter=blob:none origin {branch} && git checkout FETCH_HEAD",
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
                command: Some(vec!["sh".to_string(), "-c".to_string(), script]),
                args: None,
                env: None,
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
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutate_git_checkout_default() {
        let mut step_type = "GitCheckout".to_string();
        let mut spec = serde_json::json!({
            "repo": "https://github.com/example/repo.git"
        });

        mutate(&mut step_type, &mut spec);

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

    #[test]
    fn test_mutate_git_checkout_sparse() {
        let mut step_type = "GitCheckout".to_string();
        let mut spec = serde_json::json!({
            "repo": "https://github.com/example/repo.git",
            "ref": "v1.0",
            "depth": 5,
            "sparse_checkout": ["src/", "docs/"],
            "destination": "/workspace/repo"
        });

        mutate(&mut step_type, &mut spec);

        assert_eq!(step_type, "RunContainer");
        let spec_obj = spec.as_object().unwrap();
        let command = spec_obj.get("command").unwrap().as_array().unwrap();
        assert_eq!(
            command[2],
            "mkdir -p /workspace/repo && cd /workspace/repo && git init && git remote add origin https://github.com/example/repo.git && git sparse-checkout set src/ docs/ && git fetch --depth 5 --filter=blob:none origin v1.0 && git checkout FETCH_HEAD"
        );
    }
}
