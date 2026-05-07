use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use std::io::Read;
use stormchaser_model::BackendId;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_model::StorageBackend;
use tar::Archive;
use uuid::Uuid;

pub async fn persist_step_test_reports(
    payload: &Value,
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    step_id: Uuid,
    pool: &PgPool,
) -> Result<()> {
    if let Some(reports) = payload["test_reports"].as_object() {
        for (_key, report_val) in reports {
            let name = report_val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let file_name = report_val
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let format = report_val
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let hash = report_val
                .get("hash")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if report_val
                .get("is_claim")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let remote_path = report_val.get("remote_path").and_then(|v| v.as_str());
                let backend_id = report_val.get("backend_id").and_then(|v| {
                    if let Some(s) = v.as_str() {
                        uuid::Uuid::parse_str(s).ok().map(BackendId::new)
                    } else {
                        None
                    }
                });

                if let (Some(path), Some(bid)) = (remote_path, backend_id) {
                    // Download and parse
                    let backend: StorageBackend =
                        crate::db::storage::get_storage_backend_by_id(pool, bid.into_inner())
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("Storage backend not found"))?;

                    let client = crate::s3::get_s3_client(&backend).await?;
                    let bucket = backend.config["bucket"]
                        .as_str()
                        .context("Missing bucket")?;
                    let response = client.get_object().bucket(bucket).key(path).send().await?;
                    let data = response.body.collect().await?.to_vec();

                    // It's a tar.gz
                    let mut summaries = Vec::new();
                    let mut test_cases = Vec::new();
                    let mut raw_contents = Vec::new();

                    {
                        // Scope for !Send Archive
                        let tar_gz = GzDecoder::new(&data[..]);
                        let mut archive = Archive::new(tar_gz);
                        for entry in archive.entries()? {
                            let mut entry = entry?;
                            let mut content = String::new();
                            entry.read_to_string(&mut content)?;

                            if format == "junit" {
                                if let Ok((summary, cases)) = crate::junit::parse_junit(
                                    &content,
                                    name,
                                    RunId::new(run_id),
                                    StepInstanceId::new(step_id),
                                ) {
                                    summaries.push(summary);
                                    test_cases.extend(cases);
                                }
                            }
                            raw_contents.push(content);
                        }
                    }

                    for case in test_cases {
                        crate::db::insert_step_test_case(&mut **tx, run_id, step_id, name, &case)
                            .await?;
                    }

                    if let Some(final_summary) = crate::junit::aggregate_summaries(&summaries) {
                        crate::db::insert_step_test_summary(
                            &mut **tx,
                            run_id,
                            step_id,
                            name,
                            &final_summary,
                        )
                        .await?;
                    }

                    let combined_raw = raw_contents.join("\n---\n");

                    crate::db::insert_step_test_report(
                        &mut **tx,
                        run_id,
                        step_id,
                        name,
                        file_name,
                        format,
                        Some(&combined_raw),
                        hash,
                        Some(bid.into_inner()),
                        Some(path),
                    )
                    .await?;
                }
            } else if let Some(content) = report_val.get("content").and_then(|v| v.as_str()) {
                // Legacy in-memory report
                if format == "junit" {
                    if let Ok((summary, cases)) = crate::junit::parse_junit(
                        content,
                        name,
                        RunId::new(run_id),
                        StepInstanceId::new(step_id),
                    ) {
                        crate::db::insert_step_test_summary(
                            &mut **tx, run_id, step_id, name, &summary,
                        )
                        .await?;
                        for case in cases {
                            crate::db::insert_step_test_case(
                                &mut **tx, run_id, step_id, name, &case,
                            )
                            .await?;
                        }
                    }
                }

                crate::db::insert_step_test_report(
                    &mut **tx,
                    run_id,
                    step_id,
                    name,
                    file_name,
                    format,
                    Some(content),
                    hash,
                    None,
                    None,
                )
                .await?;
            }
        }
    }
    Ok(())
}
