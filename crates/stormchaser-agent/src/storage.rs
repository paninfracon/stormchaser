use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use tracing::{error, info, warn};
use uuid::Uuid;

pub async fn unpark_storage(
    url: &str,
    expected_hash: Option<&str>,
    destination: &str,
) -> Result<()> {
    info!("Unparking storage from {} to {}...", url, destination);
    let client = reqwest::Client::new();
    let mut response = client.get(url).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("Failed to download storage: {} - {}", status, text);
    }

    let mut hasher = Sha256::new();
    let tar_path = format!("/tmp/unpark_{}.tar.gz", Uuid::new_v4());
    {
        let mut file = File::create(&tar_path)?;
        while let Some(chunk) = response.chunk().await? {
            hasher.update(&chunk);
            file.write_all(&chunk)?;
        }
    }

    let actual_hash = hex::encode(hasher.finalize());
    if let Some(expected) = expected_hash {
        if actual_hash != expected {
            let _ = std::fs::remove_file(&tar_path);
            anyhow::bail!(
                "Hash verification failed! Expected: {}, Actual: {}",
                expected,
                actual_hash
            );
        }
        info!("Hash verification successful: {}", actual_hash);
    } else {
        warn!(
            "No expected hash provided for verification (Actual: {})",
            actual_hash
        );
    }

    info!("Extracting tarball...");
    std::fs::create_dir_all(destination)?;
    // Use tar crate properly for extraction
    let tar_gz = File::open(&tar_path)?;
    let decoder = flate2::read::GzDecoder::new(tar_gz);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(destination)?;

    let _ = std::fs::remove_file(tar_path);
    info!("Successfully unparked storage");
    Ok(())
}

pub async fn park_storage(urls: Value, paths: Value) -> Result<HashMap<String, String>> {
    let client = reqwest::Client::new();
    let mut hashes = HashMap::new();

    if let Some(url_map) = urls.as_object() {
        for (name, url_val) in url_map {
            if let Some(put_url) = url_val.get("put_url").and_then(|u| u.as_str()) {
                if let Some(mount_path) = paths.get(name).and_then(|p| p.as_str()) {
                    info!("Parking storage '{}' from path '{}'...", name, mount_path);

                    let tar_path = format!("/tmp/{}.tar.gz", name);
                    let mut hasher = Sha256::new();
                    {
                        let tar_gz = File::create(&tar_path)?;
                        let enc = GzEncoder::new(tar_gz, Compression::default());
                        let mut tar = tar::Builder::new(enc);
                        tar.append_dir_all(".", mount_path)?;
                        tar.finish()?;
                    }

                    // Calculate hash of the produced tarball
                    {
                        let mut file = File::open(&tar_path)?;
                        let mut buffer = [0; 8192];
                        loop {
                            let count = file.read(&mut buffer)?;
                            if count == 0 {
                                break;
                            }
                            hasher.update(&buffer[..count]);
                        }
                    }
                    let hash = hex::encode(hasher.finalize());
                    hashes.insert(name.clone(), hash);

                    info!("Uploading tarball to {}", put_url);
                    let file = File::open(&tar_path)?;
                    let file_size = file.metadata()?.len();
                    let file_tokio = tokio::fs::File::open(&tar_path).await?;
                    let body = reqwest::Body::from(file_tokio);

                    let res = client
                        .put(put_url)
                        .header("Content-Length", file_size)
                        .body(body)
                        .send()
                        .await?;

                    if !res.status().is_success() {
                        let status = res.status();
                        let text = res.text().await?;
                        error!("Failed to upload storage '{}': {} - {}", name, status, text);
                    } else {
                        info!(
                            "Successfully parked storage '{}' (hash: {})",
                            name, hashes[name]
                        );
                    }

                    let _ = std::fs::remove_file(tar_path);
                }
            }
        }
    }

    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_unpark_storage_success() {
        let mock_server = MockServer::start().await;
        let dir = tempdir().unwrap();
        let dest = dir.path().join("dest");

        // Create a dummy tar.gz
        let tar_dir = tempdir().unwrap();
        let file_path = tar_dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let tar_path = tar_dir.path().join("test.tar.gz");
        {
            let tar_gz = File::create(&tar_path).unwrap();
            let enc = GzEncoder::new(tar_gz, Compression::default());
            let mut tar = tar::Builder::new(enc);
            tar.append_path_with_name(&file_path, "test.txt").unwrap();
            tar.finish().unwrap();
        }

        let tar_bytes = std::fs::read(&tar_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&tar_bytes);
        let expected_hash = hex::encode(hasher.finalize());

        Mock::given(method("GET"))
            .and(path("/storage.tar.gz"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tar_bytes))
            .mount(&mock_server)
            .await;

        let url = format!("{}/storage.tar.gz", mock_server.uri());
        unpark_storage(&url, Some(&expected_hash), dest.to_str().unwrap())
            .await
            .unwrap();

        assert!(dest.join("test.txt").exists());
        let content = std::fs::read_to_string(dest.join("test.txt")).unwrap();
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn test_park_storage_success() {
        let mock_server = MockServer::start().await;
        let dir = tempdir().unwrap();
        let mount_path = dir.path().join("storage1");
        std::fs::create_dir_all(&mount_path).unwrap();
        std::fs::write(mount_path.join("file.txt"), "data").unwrap();

        Mock::given(method("PUT"))
            .and(path("/upload/storage1.tar.gz"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let urls = json!({
            "storage1": {
                "put_url": format!("{}/upload/storage1.tar.gz", mock_server.uri())
            }
        });
        let paths = json!({
            "storage1": mount_path.to_str().unwrap()
        });

        let hashes = park_storage(urls, paths).await.unwrap();
        assert!(hashes.contains_key("storage1"));
    }
}
