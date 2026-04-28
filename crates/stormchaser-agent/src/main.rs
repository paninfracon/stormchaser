use anyhow::Result;
use clap::{Parser, Subcommand};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::process::Command;
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(author, about, long_about = None)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (rev: ", env!("VERGEN_GIT_SHA"), ", branch: ", env!("VERGEN_GIT_BRANCH"), ", built: ", env!("VERGEN_BUILD_TIMESTAMP"), ")"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Runs a user command and then parks (uploads) the SFS storage
    Run {
        /// The JSON map of storage name to parking (put) URL
        #[arg(short, long)]
        parking_urls: String,

        /// The mount path for each storage (JSON map: name -> path)
        #[arg(short, long)]
        mount_paths: String,

        /// The JSON map of artifact name to {put_url, path}
        #[arg(short, long)]
        artifact_urls: Option<String>,

        /// The JSON list of test reports (name, path, format)
        #[arg(short, long, env = "STORMCHASER_TEST_REPORTS")]
        test_reports: Option<String>,

        /// The JSON map of report name to {put_url, remote_path, backend_id}
        #[arg(short, long, env = "STORMCHASER_REPORT_URLS")]
        report_urls: Option<String>,

        /// The user command to run
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Downloads and extracts a storage tarball with hash verification
    Unpark {
        /// The download (get) URL
        #[arg(short, long)]
        url: String,

        /// The expected SHA-256 hash (hex)
        #[arg(short, long)]
        expected_hash: Option<String>,

        /// The destination directory to extract to
        #[arg(short, long)]
        destination: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    run_agent(cli).await
}

pub async fn run_agent(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Unpark {
            url,
            expected_hash,
            destination,
        } => {
            unpark_storage(&url, expected_hash.as_deref(), &destination).await?;
            Ok(())
        }
        Commands::Run {
            parking_urls,
            mount_paths,
            artifact_urls,
            test_reports,
            report_urls,
            command,
        } => {
            let urls: serde_json::Value = serde_json::from_str(&parking_urls)?;
            let paths: serde_json::Value = serde_json::from_str(&mount_paths)?;

            if command.is_empty() {
                anyhow::bail!("No command provided to run");
            }

            info!("Running user command: {:?}", command);
            let mut child = Command::new(&command[0]).args(&command[1..]).spawn()?;

            let status = child.wait()?;
            info!("User command finished with status: {}", status);

            // Always collect reports even if command failed (test failures are common)
            if let Some(reports_json) = test_reports {
                let reports: serde_json::Value = serde_json::from_str(&reports_json)?;
                let urls_val: Option<serde_json::Value> =
                    report_urls.and_then(|u| serde_json::from_str(&u).ok());
                let collected_reports = collect_test_reports(reports, urls_val).await?;
                if !collected_reports.is_empty() {
                    info!("Collected test reports: {:?}", collected_reports.keys());
                    let reports_out = serde_json::to_string(&collected_reports)?;
                    info!("Collected test reports JSON: {}", reports_out);
                    std::fs::write("/tmp/stormchaser_test_reports.json", reports_out)?;
                }
            }

            if status.success() {
                let hashes = park_storage(urls, paths).await?;
                // Print hashes to stdout for the runner to capture if needed
                if !hashes.is_empty() {
                    // Write hashes to a known file for the runner to read
                    let hashes_json = serde_json::to_string(&hashes)?;
                    info!("Parked storage hashes: {}", hashes_json);
                    std::fs::write("/tmp/stormchaser_storage_hashes.json", hashes_json)?;
                }

                if let Some(artifacts_json) = artifact_urls {
                    let artifacts: serde_json::Value = serde_json::from_str(&artifacts_json)?;
                    let artifact_meta = park_artifacts(artifacts).await?;
                    if !artifact_meta.is_empty() {
                        let meta_json = serde_json::to_string(&artifact_meta)?;
                        info!("Parked artifacts: {}", meta_json);
                        std::fs::write("/tmp/stormchaser_artifact_meta.json", meta_json)?;
                    }
                }
            } else {
                info!("User command failed, skipping storage and artifact parking");
            }

            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

async fn unpark_storage(url: &str, expected_hash: Option<&str>, destination: &str) -> Result<()> {
    info!("Unparking storage from {} to {}...", url, destination);
    let client = reqwest::Client::new();
    let mut response = client.get(url).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("Failed to download storage: {} - {}", status, text);
    }

    let mut hasher = Sha256::new();
    let tar_path = format!("/tmp/unpark_{}.tar.gz", uuid::Uuid::new_v4());
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

async fn park_storage(
    urls: serde_json::Value,
    paths: serde_json::Value,
) -> Result<std::collections::HashMap<String, String>> {
    let client = reqwest::Client::new();
    let mut hashes = std::collections::HashMap::new();

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

async fn park_artifacts(
    artifacts: serde_json::Value,
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    let client = reqwest::Client::new();
    let mut metadata_map = std::collections::HashMap::new();

    if let Some(artifact_map) = artifacts.as_object() {
        for (name, artifact_val) in artifact_map {
            let backend_type = artifact_val
                .get("backend_type")
                .and_then(|t| t.as_str())
                .unwrap_or("s3");

            let path = match artifact_val.get("path").and_then(|p| p.as_str()) {
                Some(p) => p,
                None => continue,
            };

            if !std::path::Path::new(path).exists() {
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

            if backend_type == "s3" {
                if let Some(put_url) = artifact_val.get("put_url").and_then(|u| u.as_str()) {
                    let file_tokio = tokio::fs::File::open(path).await?;
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
                        error!(
                            "Failed to upload artifact '{}': {} - {}",
                            name, status, text
                        );
                    } else {
                        info!("Successfully parked artifact '{}' (hash: {})", name, hash);
                        metadata_map.insert(
                            name.clone(),
                            serde_json::json!({
                                "hash": hash,
                                "size": file_size,
                                "content_type": "application/octet-stream", // Default
                            }),
                        );
                    }
                } else {
                    warn!("Missing 'put_url' for S3 artifact '{}'", name);
                }
            } else if backend_type == "oci" {
                if let Some(remote_path) = artifact_val.get("remote_path").and_then(|u| u.as_str())
                {
                    let mut cmd = std::process::Command::new("oras");
                    cmd.arg("push");

                    if let (Some(user), Some(pass)) = (
                        artifact_val.get("username").and_then(|u| u.as_str()),
                        artifact_val.get("password").and_then(|p| p.as_str()),
                    ) {
                        cmd.arg("--username").arg(user).arg("--password").arg(pass);
                    }

                    cmd.arg(remote_path);
                    cmd.arg(path);

                    let output = cmd.output()?;
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
                            serde_json::json!({
                                "hash": hash,
                                "size": file_size,
                                "content_type": "application/vnd.oci.image.layer.v1.tar+gzip",
                            }),
                        );
                    }
                } else {
                    warn!("Missing 'remote_path' for OCI artifact '{}'", name);
                }
            } else {
                warn!("Unsupported artifact backend_type '{}'", backend_type);
            }
        }
    }

    Ok(metadata_map)
}

async fn collect_test_reports(
    reports: serde_json::Value,
    urls: Option<serde_json::Value>,
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    let mut collected = std::collections::HashMap::new();
    let client = reqwest::Client::new();

    if let Some(report_list) = reports.as_array() {
        for report in report_list {
            if let (Some(name), Some(path), Some(format)) = (
                report.get("name").and_then(|v| v.as_str()),
                report.get("path").and_then(|v| v.as_str()),
                report.get("format").and_then(|v| v.as_str()),
            ) {
                // Support globbing
                let entries = glob::glob(path)?;
                let mut matched_files = Vec::new();
                for p in entries.flatten() {
                    if p.is_file() {
                        matched_files.push(p);
                    }
                }

                if matched_files.is_empty() {
                    continue;
                }

                let put_url = urls
                    .as_ref()
                    .and_then(|u| u.get(name))
                    .and_then(|r| r.get("put_url"))
                    .and_then(|v| v.as_str());

                let remote_path = urls
                    .as_ref()
                    .and_then(|u| u.get(name))
                    .and_then(|r| r.get("remote_path"))
                    .and_then(|v| v.as_str());

                let backend_id = urls
                    .as_ref()
                    .and_then(|u| u.get(name))
                    .and_then(|r| r.get("backend_id"))
                    .and_then(|v| v.as_str());

                if let Some(url) = put_url {
                    // Zip the files
                    let tar_path = format!("/tmp/report_{}_{}.tar.gz", name, uuid::Uuid::new_v4());
                    let file = File::create(&tar_path)?;
                    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                    let mut tar = tar::Builder::new(enc);

                    for p in &matched_files {
                        let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
                        tar.append_path_with_name(p, file_name)?;
                    }
                    tar.finish()?;
                    drop(tar);

                    // Upload
                    let file_size = std::fs::metadata(&tar_path)?.len();
                    let file_tokio = tokio::fs::File::open(&tar_path).await?;
                    let body = reqwest::Body::from(file_tokio);

                    let res = client
                        .put(url)
                        .header("Content-Length", file_size)
                        .body(body)
                        .send()
                        .await?;

                    if res.status().is_success() {
                        let mut hasher = Sha256::new();
                        let mut f = File::open(&tar_path)?;
                        std::io::copy(&mut f, &mut hasher)?;
                        let hash = hex::encode(hasher.finalize());

                        collected.insert(
                            name.to_string(),
                            serde_json::json!({
                                "name": name,
                                "file_name": format!("{}.tar.gz", name),
                                "format": format,
                                "hash": hash,
                                "remote_path": remote_path,
                                "backend_id": backend_id,
                                "is_claim": true,
                            }),
                        );
                    } else {
                        error!("Failed to upload report '{}': {}", name, res.status());
                    }
                } else {
                    // Fallback to in-memory if no put_url (legacy or small reports)
                    for p in matched_files {
                        let mut file = File::open(&p)?;
                        let mut content = String::new();
                        file.read_to_string(&mut content)?;

                        let mut hasher = Sha256::new();
                        hasher.update(content.as_bytes());
                        let hash = hex::encode(hasher.finalize());

                        let file_name = p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(name)
                            .to_string();

                        collected.insert(
                            format!("{}_{}", name, file_name),
                            serde_json::json!({
                                "name": name,
                                "file_name": file_name,
                                "format": format,
                                "content": content,
                                "hash": hash,
                            }),
                        );
                    }
                }
            }
        }
    }

    Ok(collected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_collect_test_reports() {
        let dir = tempdir().unwrap();
        let report_file = dir.path().join("test-report.xml");
        std::fs::write(&report_file, "<testsuite></testsuite>").unwrap();

        let reports = json!([
            {
                "name": "junit",
                "path": report_file.to_str().unwrap(),
                "format": "junit"
            }
        ]);

        let collected = collect_test_reports(reports, None).await.unwrap();
        assert_eq!(collected.len(), 1);
        let key = format!(
            "junit_{}",
            report_file.file_name().unwrap().to_str().unwrap()
        );
        assert!(collected.contains_key(&key));
        assert_eq!(collected[&key]["format"], "junit");
        assert_eq!(collected[&key]["content"], "<testsuite></testsuite>");
    }

    #[tokio::test]
    async fn test_collect_test_reports_upload() {
        let mock_server = MockServer::start().await;
        let dir = tempdir().unwrap();
        let report_file = dir.path().join("test-report.xml");
        std::fs::write(&report_file, "<testsuite></testsuite>").unwrap();

        Mock::given(method("PUT"))
            .and(path("/upload/report.tar.gz"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let reports = json!([
            {
                "name": "api-tests",
                "path": report_file.to_str().unwrap(),
                "format": "junit"
            }
        ]);

        let urls = json!({
            "api-tests": {
                "put_url": format!("{}/upload/report.tar.gz", mock_server.uri()),
                "remote_path": "path/to/report.tar.gz",
                "backend_id": "backend-id"
            }
        });

        let collected = collect_test_reports(reports, Some(urls)).await.unwrap();
        assert_eq!(collected.len(), 1);
        assert!(collected.contains_key("api-tests"));
        assert_eq!(collected["api-tests"]["is_claim"], true);
        assert_eq!(collected["api-tests"]["file_name"], "api-tests.tar.gz");
    }

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

    #[tokio::test]
    async fn test_park_artifacts_success() {
        let mock_server = MockServer::start().await;
        let dir = tempdir().unwrap();
        let artifact_path = dir.path().join("artifact.bin");
        std::fs::write(&artifact_path, "binary content").unwrap();

        Mock::given(method("PUT"))
            .and(path("/artifacts/artifact.bin"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let artifacts = json!({
            "my-artifact": {
                "backend_type": "s3",
                "path": artifact_path.to_str().unwrap(),
                "put_url": format!("{}/artifacts/artifact.bin", mock_server.uri())
            }
        });

        let metadata = park_artifacts(artifacts).await.unwrap();
        assert!(metadata.contains_key("my-artifact"));
        assert_eq!(metadata["my-artifact"]["size"], 14);
    }

    #[test]
    fn test_cli_parsing_unpark() {
        let args = vec![
            "stormchaser-agent",
            "unpark",
            "--url",
            "http://example.com/data.tar.gz",
            "--expected-hash",
            "abcdef123456",
            "--destination",
            "/data",
        ];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Unpark {
                url,
                expected_hash,
                destination,
            } => {
                assert_eq!(url, "http://example.com/data.tar.gz");
                assert_eq!(expected_hash.unwrap(), "abcdef123456");
                assert_eq!(destination, "/data");
            }
            _ => panic!("Expected Unpark command"),
        }
    }

    #[test]
    fn test_cli_parsing_run() {
        let args = vec![
            "stormchaser-agent",
            "run",
            "--parking-urls",
            "{}",
            "--mount-paths",
            "{}",
            "--",
            "echo",
            "hello",
        ];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run {
                parking_urls,
                mount_paths,
                command,
                ..
            } => {
                assert_eq!(parking_urls, "{}");
                assert_eq!(mount_paths, "{}");
                assert_eq!(command, vec!["echo", "hello"]);
            }
            _ => panic!("Expected Run command"),
        }
    }
}
