use anyhow::Result;
use flate2::{write::GzEncoder, Compression};
use glob::glob;
use reqwest::{Body, Client};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use tar::Builder;
use tokio::fs::File as AsyncFile;
use tracing::{error, warn};
use uuid::Uuid;

struct UploadReportParams<'a> {
    name: &'a str,
    format: &'a str,
    matched_files: &'a [PathBuf],
    url: &'a str,
    remote_path: Option<&'a str>,
    connection_id: Option<&'a str>,
    client: &'a Client,
}

async fn upload_report(
    params: UploadReportParams<'_>,
    collected: &mut HashMap<String, Value>,
) -> Result<()> {
    // Zip the files
    let tar_path = format!("/tmp/report_{}_{}.tar.gz", params.name, Uuid::new_v4());
    let file = File::create(&tar_path)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(enc);

    for p in params.matched_files {
        let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        tar.append_path_with_name(p, file_name)?;
    }
    tar.finish()?;
    drop(tar);

    // Upload
    let file_size = fs::metadata(&tar_path)?.len();
    let file_tokio = AsyncFile::open(&tar_path).await?;
    let body = Body::from(file_tokio);

    let res = params
        .client
        .put(params.url)
        .header("Content-Length", file_size)
        .body(body)
        .send()
        .await?;

    if res.status().is_success() {
        let mut hasher = Sha256::new();
        let mut f = File::open(&tar_path)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let hash = hex::encode(hasher.finalize());

        collected.insert(
            params.name.to_string(),
            json!({
                "name": params.name,
                "file_name": format!("{}.tar.gz", params.name),
                "format": params.format,
                "hash": hash,
                "remote_path": params.remote_path,
                "connection_id": params.connection_id,
                "is_claim": true,
            }),
        );
    } else {
        error!(
            "Failed to upload report '{}': {}",
            params.name,
            res.status()
        );
    }

    Ok(())
}

fn fallback_to_memory(
    name: &str,
    format: &str,
    matched_files: Vec<PathBuf>,
    collected: &mut HashMap<String, Value>,
) -> Result<()> {
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
            json!({
                "name": name,
                "file_name": file_name,
                "format": format,
                "content": content,
                "hash": hash,
            }),
        );
    }
    Ok(())
}

/// Collects test reports from the filesystem and optionally uploads them.
///
/// With `roots`, only files that resolve inside them (the step's storage volumes) are
/// collected: the agent is then a helper that can read files the step cannot.
pub async fn collect_test_reports(
    reports: Value,
    urls: Option<Value>,
    roots: Option<&crate::Roots>,
) -> Result<HashMap<String, Value>> {
    let mut collected = HashMap::new();
    let client = crate::http_client()?;

    if let Some(report_list) = reports.as_array() {
        for report in report_list {
            if let (Some(name), Some(path), Some(format)) = (
                report.get("name").and_then(|v| v.as_str()),
                report.get("path").and_then(|v| v.as_str()),
                report.get("format").and_then(|v| v.as_str()),
            ) {
                // Support globbing
                let entries = glob(path)?;
                let mut matched_files = Vec::new();
                for p in entries.flatten() {
                    let p = match roots.map(|r| r.resolve(&p)) {
                        None => p,
                        Some(Ok(resolved)) => resolved,
                        Some(Err(e)) => {
                            warn!("Not collecting report file: {:#}", e);
                            continue;
                        }
                    };
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

                let connection_id = urls
                    .as_ref()
                    .and_then(|u| u.get(name))
                    .and_then(|r| r.get("connection_id"))
                    .and_then(|v| v.as_str());

                if let Some(url) = put_url {
                    upload_report(
                        UploadReportParams {
                            name,
                            format,
                            matched_files: &matched_files,
                            url,
                            remote_path,
                            connection_id,
                            client: &client,
                        },
                        &mut collected,
                    )
                    .await?;
                } else {
                    fallback_to_memory(name, format, matched_files, &mut collected)?;
                }
            }
        }
    }

    Ok(collected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_collect_test_reports() {
        let dir = tempdir().unwrap();
        let report_file = dir.path().join("test-report.xml");
        fs::write(&report_file, "<testsuite></testsuite>").unwrap();

        let reports = json!([
            {
                "name": "junit",
                "path": report_file.to_str().unwrap(),
                "format": "junit"
            }
        ]);

        let collected = collect_test_reports(reports, None, None).await.unwrap();
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
        fs::write(&report_file, "<testsuite></testsuite>").unwrap();

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
                "connection_id": "backend-id"
            }
        });

        let collected = collect_test_reports(reports, Some(urls), None)
            .await
            .unwrap();
        assert_eq!(collected.len(), 1);
        assert!(collected.contains_key("api-tests"));
        assert_eq!(collected["api-tests"]["is_claim"], true);
        assert_eq!(collected["api-tests"]["file_name"], "api-tests.tar.gz");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_outside_the_mounts_are_not_collected_by_a_helper() {
        let mount = tempdir().unwrap();
        let helper_only = tempdir().unwrap();
        fs::write(helper_only.path().join("config.json"), "push credentials").unwrap();
        fs::write(mount.path().join("ok.xml"), "<testsuite></testsuite>").unwrap();
        std::os::unix::fs::symlink(
            helper_only.path().join("config.json"),
            mount.path().join("leak.xml"),
        )
        .unwrap();

        let reports = json!([
            {"name": "inside", "path": mount.path().join("*.xml").to_str().unwrap(), "format": "junit"},
            {"name": "outside", "path": helper_only.path().join("*").to_str().unwrap(), "format": "junit"}
        ]);
        let roots = crate::Roots::from_mount_paths(&json!({"ws": mount.path()}));
        let collected = collect_test_reports(reports.clone(), None, Some(&roots))
            .await
            .unwrap();
        let all = serde_json::to_string(&collected).unwrap();
        assert!(!all.contains("push credentials"), "{}", all);
        assert_eq!(collected.keys().collect::<Vec<_>>(), ["inside_ok.xml"]);

        // An agent inside the step's own container may read anywhere
        let collected = collect_test_reports(reports, None, None).await.unwrap();
        assert!(collected.contains_key("outside_config.json"));
    }
}
