use anyhow::Result;
use reqwest::{Body, Client};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use tokio::fs::File as AsyncFile;
use tracing::{error, info, warn};

async fn upload_to_s3(
    name: &String,
    artifact_val: &Value,
    path: &str,
    file_size: u64,
    hash: String,
    client: &Client,
    metadata_map: &mut HashMap<String, Value>,
) {
    if let Some(put_url) = artifact_val.get("put_url").and_then(|u| u.as_str()) {
        let file_tokio = match AsyncFile::open(path).await {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open artifact file '{}': {}", name, e);
                return;
            }
        };
        let body = Body::from(file_tokio);

        let res = match client
            .put(put_url)
            .header("Content-Length", file_size)
            .body(body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("Request failed for artifact '{}': {}", name, e);
                return;
            }
        };

        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            error!(
                "Failed to upload artifact '{}': {} - {}",
                name, status, text
            );
        } else {
            info!("Successfully parked artifact '{}' (hash: {})", name, hash);
            metadata_map.insert(
                name.clone(),
                json!({
                    "hash": hash,
                    "size": file_size,
                    "content_type": "application/octet-stream",
                }),
            );
        }
    } else {
        warn!("Missing 'put_url' for S3 artifact '{}'", name);
    }
}

async fn upload_to_oci(
    name: &String,
    artifact_val: &Value,
    path: &str,
    file_size: u64,
    hash: String,
    registry_config: Option<&str>,
    metadata_map: &mut HashMap<String, Value>,
) {
    if let Some(remote_path) = artifact_val.get("remote_path").and_then(|u| u.as_str()) {
        let mut cmd = Command::new("oras");
        cmd.arg("push");

        if let (Some(user), Some(pass)) = (
            artifact_val.get("username").and_then(|u| u.as_str()),
            artifact_val.get("password").and_then(|p| p.as_str()),
        ) {
            cmd.arg("--username").arg(user).arg("--password").arg(pass);
        }

        // Credentials the runner placed for this push, outside the step's reach
        if let Some(config) = registry_config.filter(|c| Path::new(c).is_file()) {
            cmd.arg("--registry-config").arg(config);
        }
        if artifact_val.get("plain_http").and_then(|p| p.as_bool()) == Some(true) {
            cmd.arg("--plain-http");
        }

        // oras refuses absolute paths and names the layer after the path it is given, so
        // push the file by name from its own directory.
        let (dir, file_name) = split_for_push(path);
        cmd.current_dir(dir);
        cmd.arg(remote_path);
        cmd.arg(file_name);

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to execute oras for artifact '{}': {}", name, e);
                return;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Failed to push OCI artifact '{}': {}", name, stderr);
        } else {
            info!(
                "Successfully parked OCI artifact '{}' (hash: {})",
                name, hash
            );
            metadata_map.insert(
                name.clone(),
                json!({
                    "hash": hash,
                    "size": file_size,
                    "content_type": "application/vnd.oci.image.layer.v1.tar+gzip",
                }),
            );
        }
    } else {
        warn!("Missing 'remote_path' for OCI artifact '{}'", name);
    }
}

/// Splits an artifact path into the directory to push from and the name to push.
fn split_for_push(path: &str) -> (&Path, &std::ffi::OsStr) {
    let path = Path::new(path);
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) if !dir.as_os_str().is_empty() => (dir, name),
        (_, Some(name)) => (Path::new("."), name),
        _ => (Path::new("."), path.as_os_str()),
    }
}

/// Uploads artifacts to configured backends and returns metadata maps.
///
/// `registry_config` is a Docker client configuration file with the credentials for OCI
/// pushes, if the runner provided one.
pub async fn park_artifacts(
    artifacts: Value,
    registry_config: Option<&str>,
) -> Result<HashMap<String, Value>> {
    let client = crate::http_client()?;
    let mut metadata_map = HashMap::new();

    if let Some(artifact_map) = artifacts.as_object() {
        for (name, artifact_val) in artifact_map {
            let connection_type = artifact_val
                .get("connection_type")
                .and_then(|t| t.as_str())
                .unwrap_or("s3");

            let path = match artifact_val.get("path").and_then(|p| p.as_str()) {
                Some(p) => p,
                None => continue,
            };

            if !Path::new(path).exists() {
                warn!("Artifact '{}' path '{}' not found, skipping", name, path);
                continue;
            }

            info!("Parking artifact '{}' from path '{}'...", name, path);

            let file = File::open(path)?;
            let file_size = file.metadata()?.len();
            let mut hasher = Sha256::new();
            {
                let mut file_read = File::open(path)?;
                let mut buffer = [0; 8192];
                loop {
                    let count = file_read.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    hasher.update(&buffer[..count]);
                }
            }
            let hash = hex::encode(hasher.finalize());

            if connection_type == "s3" {
                upload_to_s3(
                    name,
                    artifact_val,
                    path,
                    file_size,
                    hash,
                    &client,
                    &mut metadata_map,
                )
                .await;
            } else if connection_type == "oci" {
                upload_to_oci(
                    name,
                    artifact_val,
                    path,
                    file_size,
                    hash,
                    registry_config,
                    &mut metadata_map,
                )
                .await;
            } else {
                warn!("Unsupported artifact connection_type '{}'", connection_type);
            }
        }
    }

    Ok(metadata_map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_paths_are_pushed_by_name_from_their_directory() {
        let (dir, name) = split_for_push("/workspace/bin/app");
        assert_eq!(
            (dir, name.to_str().unwrap()),
            (Path::new("/workspace/bin"), "app")
        );
        // A trailing slash names a directory artifact
        let (dir, name) = split_for_push("/workspace/dist/");
        assert_eq!(
            (dir, name.to_str().unwrap()),
            (Path::new("/workspace"), "dist")
        );
        let (dir, name) = split_for_push("app.bin");
        assert_eq!((dir, name.to_str().unwrap()), (Path::new("."), "app.bin"));
    }
    use std::fs;
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_park_artifacts_success() {
        let mock_server = MockServer::start().await;
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("artifact.bin");
        fs::write(&artifact_path, "binary content").unwrap();

        Mock::given(method("PUT"))
            .and(path("/artifacts/artifact.bin"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let artifacts = json!({
            "my-artifact": {
                "connection_type": "s3",
                "path": artifact_path.to_str().unwrap(),
                "put_url": format!("{}/artifacts/artifact.bin", mock_server.uri())
            }
        });

        let metadata = park_artifacts(artifacts, None).await.unwrap();
        assert!(metadata.contains_key("my-artifact"));
        assert_eq!(metadata["my-artifact"]["size"], 14);
    }
}
