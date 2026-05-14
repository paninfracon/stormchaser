use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use std::io::Read;
use stormchaser_model::Connection;
use stormchaser_model::ConnectionId;
use tar::Archive;

use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
async fn process_claim_report(
    report_val: &Value,
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    pool: &PgPool,
    name: &str,
    file_name: &str,
    format: &str,
    hash: &str,
) -> Result<()> {
    let remote_path = report_val.get("remote_path").and_then(|v| v.as_str());
    let connection_id = report_val.get("connection_id").and_then(|v| {
        if let Some(s) = v.as_str() {
            uuid::Uuid::parse_str(s).ok().map(ConnectionId::new)
        } else {
            None
        }
    });

    if let (Some(path), Some(bid)) = (remote_path, connection_id) {
        // Download and parse
        let backend: Connection =
            crate::db::connections::get_storage_backend_by_id(pool, bid.into_inner())
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
                    if let Ok((summary, cases)) =
                        crate::junit::parse_junit(&content, name, run_id, step_id)
                    {
                        summaries.push(summary);
                        test_cases.extend(cases);
                    }
                }
                raw_contents.push(content);
            }
        }

        for case in test_cases {
            crate::db::insert_step_test_case(
                &mut **tx,
                run_id.into_inner(),
                step_id.into_inner(),
                name,
                &case,
            )
            .await?;
        }

        if let Some(final_summary) = crate::junit::aggregate_summaries(&summaries) {
            crate::db::insert_step_test_summary(
                &mut **tx,
                run_id.into_inner(),
                step_id.into_inner(),
                name,
                &final_summary,
            )
            .await?;
        }

        let combined_raw = raw_contents.join("\n---\n");

        crate::db::insert_step_test_report(
            &mut **tx,
            run_id.into_inner(),
            step_id.into_inner(),
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
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn process_legacy_report(
    report_val: &Value,
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    name: &str,
    file_name: &str,
    format: &str,
    hash: &str,
) -> Result<()> {
    if let Some(content) = report_val.get("content").and_then(|v| v.as_str()) {
        // Legacy in-memory report
        if format == "junit" {
            if let Ok((summary, cases)) = crate::junit::parse_junit(content, name, run_id, step_id)
            {
                crate::db::insert_step_test_summary(
                    &mut **tx,
                    run_id.into_inner(),
                    step_id.into_inner(),
                    name,
                    &summary,
                )
                .await?;
                for case in cases {
                    crate::db::insert_step_test_case(
                        &mut **tx,
                        run_id.into_inner(),
                        step_id.into_inner(),
                        name,
                        &case,
                    )
                    .await?;
                }
            }
        }

        crate::db::insert_step_test_report(
            &mut **tx,
            run_id.into_inner(),
            step_id.into_inner(),
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
    Ok(())
}

pub async fn persist_step_test_reports(
    test_reports: Option<&HashMap<String, Value>>,
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    pool: &PgPool,
) -> Result<()> {
    if let Some(reports) = test_reports {
        for report_val in reports.values() {
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
                process_claim_report(
                    report_val, tx, run_id, step_id, pool, name, file_name, format, hash,
                )
                .await?;
            } else {
                process_legacy_report(
                    report_val, tx, run_id, step_id, name, file_name, format, hash,
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests_helpers {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_process_legacy_report() {
        // Dummy test to exercise the new method
        let _val = serde_json::json!({"content": "foo"});
        // Without real tx and pool, we just check compilation
        let _f = process_legacy_report;
        let _g = process_claim_report;
    }
}
