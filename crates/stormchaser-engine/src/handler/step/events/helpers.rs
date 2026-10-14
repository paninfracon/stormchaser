use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::io::Read;
use stormchaser_model::dsl::TestReport;
use stormchaser_model::Connection;
use tar::Archive;

use std::collections::{HashMap, HashSet};
use stormchaser_model::step::StepStatus;

use crate::handler::step::dispatch::{find_step, test_report_key};

/// Largest report archive the engine downloads from storage.
const MAX_REPORT_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
/// Most data the engine unpacks from one report archive, across all its files.
const MAX_REPORT_UNPACKED_BYTES: u64 = 64 * 1024 * 1024;
/// Most files the engine reads from one report archive.
const MAX_REPORT_FILES: usize = 1000;
/// Most report text the engine accepts inline in a single step event.
const MAX_INLINE_REPORT_BYTES: usize = 4 * 1024 * 1024;

/// Unpacks a gzipped tar of report files, refusing to expand past the limits above.
///
/// The archive was uploaded by the step, so its size, compression ratio and entry count
/// are whatever the workflow author wants them to be.
fn unpack_report_archive(data: &[u8]) -> Result<Vec<String>> {
    let mut archive = Archive::new(GzDecoder::new(data));
    let mut contents = Vec::new();
    let mut remaining = MAX_REPORT_UNPACKED_BYTES;

    for entry in archive.entries()? {
        let entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        if contents.len() >= MAX_REPORT_FILES {
            anyhow::bail!("archive holds more than {} files", MAX_REPORT_FILES);
        }

        let mut content = String::new();
        // One byte past the budget, so reaching it can be told apart from fitting exactly
        let read = entry.take(remaining + 1).read_to_string(&mut content)? as u64;
        if read > remaining {
            anyhow::bail!(
                "archive unpacks to more than {} bytes",
                MAX_REPORT_UNPACKED_BYTES
            );
        }
        remaining -= read;
        // PostgreSQL text cannot hold NUL
        contents.push(content.replace('\0', ""));
    }
    Ok(contents)
}

/// Downloads the archive a step uploaded for one of its reports.
///
/// Both the storage backend and the key are worked out here, the same way they were when
/// the upload URL was issued. The step's event is never asked where the archive is:
/// otherwise a step could have the engine fetch any object from any storage connection
/// and publish it as a report.
async fn fetch_report_archive(pool: &PgPool, key: &str) -> Result<(Connection, Vec<u8>)> {
    let backend: Connection = crate::db::get_default_sfs_backend(pool)
        .await?
        .context("no default SFS backend")?;
    let client = crate::s3::get_s3_client(&backend).await?;
    let bucket = backend.config["bucket"]
        .as_str()
        .context("Missing bucket")?;
    let response = client.get_object().bucket(bucket).key(key).send().await?;

    if response
        .content_length()
        .is_some_and(|len| len > MAX_REPORT_ARCHIVE_BYTES as i64)
    {
        anyhow::bail!("archive is larger than {} bytes", MAX_REPORT_ARCHIVE_BYTES);
    }

    let mut data = Vec::new();
    let mut body = response.body;
    while let Some(chunk) = body.try_next().await? {
        if data.len() + chunk.len() > MAX_REPORT_ARCHIVE_BYTES {
            anyhow::bail!("archive is larger than {} bytes", MAX_REPORT_ARCHIVE_BYTES);
        }
        data.extend_from_slice(&chunk);
    }
    Ok((backend, data))
}

async fn process_claim_report(
    report: &TestReport,
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    pool: &PgPool,
) -> Result<()> {
    let name = report.name.as_str();
    let key = test_report_key(run_id, step_id, name);

    // A missing, oversized or malformed archive costs the step its report, not its result
    let (backend, data) = match fetch_report_archive(pool, &key).await {
        Ok(fetched) => fetched,
        Err(e) => {
            tracing::warn!(
                "Skipping test report '{}' of step {}: {:#}",
                name,
                step_id,
                e
            );
            return Ok(());
        }
    };
    let raw_contents = match unpack_report_archive(&data) {
        Ok(contents) => contents,
        Err(e) => {
            tracing::warn!(
                "Skipping test report '{}' of step {}: {:#}",
                name,
                step_id,
                e
            );
            return Ok(());
        }
    };
    let hash = hex::encode(Sha256::digest(&data));

    let mut summaries = Vec::new();
    let mut test_cases = Vec::new();
    if report.format == "junit" {
        for content in &raw_contents {
            if let Ok((summary, cases)) = crate::junit::parse_junit(content, name, run_id, step_id)
            {
                summaries.push(summary);
                test_cases.extend(cases);
            }
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
        &format!("{}.tar.gz", name),
        &report.format,
        Some(&combined_raw),
        &hash,
        Some(backend.id.into_inner()),
        Some(&key),
    )
    .await?;
    Ok(())
}

async fn process_legacy_report(
    content: &str,
    file_name: &str,
    report: &TestReport,
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
) -> Result<()> {
    let name = report.name.as_str();
    let content = content.replace('\0', "");
    let file_name: String = file_name.chars().filter(|c| *c != '\0').take(255).collect();
    let hash = hex::encode(Sha256::digest(content.as_bytes()));

    if report.format == "junit" {
        if let Ok((summary, cases)) = crate::junit::parse_junit(&content, name, run_id, step_id) {
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
        &file_name,
        &report.format,
        Some(&content),
        &hash,
        None,
        None,
    )
    .await?;
    Ok(())
}

pub fn is_terminal_step_status(status: &StepStatus) -> bool {
    matches!(
        *status,
        StepStatus::Succeeded
            | StepStatus::Failed
            | StepStatus::FailedIgnored
            | StepStatus::Skipped
            | StepStatus::Aborted
            | StepStatus::LostZombie
    )
}

/// The reports a step declares in its workflow, which are the only ones it may hand in.
///
/// A run whose stored definition cannot be read declares none.
pub async fn declared_test_reports(
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_name: &str,
) -> Vec<TestReport> {
    let workflow = match crate::handler::fetch_run_context(run_id, &mut **tx).await {
        Ok(context) => {
            serde_json::from_value::<stormchaser_dsl::ast::Workflow>(context.workflow_definition)
                .map_err(anyhow::Error::from)
        }
        Err(e) => Err(e),
    };
    match workflow {
        Ok(workflow) => find_step(&workflow.steps, step_name)
            .map(|step| step.reports.clone())
            .unwrap_or_default(),
        Err(e) => {
            tracing::warn!(
                "Cannot read the workflow of run {} to check its test reports: {:#}",
                run_id,
                e
            );
            Vec::new()
        }
    }
}

/// Picks the declared report a reported entry is for, if it is for one.
fn declared_report<'a>(report_val: &Value, declared: &'a [TestReport]) -> Option<&'a TestReport> {
    let name = report_val.get("name").and_then(|v| v.as_str())?;
    // The name becomes part of a storage key
    if name.contains("..") {
        return None;
    }
    declared.iter().find(|r| r.name == name)
}

/// Stores the test reports a finished step handed in.
///
/// The event comes from a runner, and its report entries come from the step's own
/// container, so they are treated as claims by the workflow author. Only reports the step
/// declares are accepted; where an uploaded report lives, its format and its checksum are
/// decided here and never read from the event.
pub async fn persist_step_test_reports(
    test_reports: Option<&HashMap<String, Value>>,
    declared: &[TestReport],
    tx: &mut Transaction<'_, Postgres>,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    pool: &PgPool,
) -> Result<()> {
    let Some(reports) = test_reports else {
        return Ok(());
    };

    let mut uploaded = HashSet::new();
    let mut inline_bytes = 0usize;

    for report_val in reports.values() {
        let Some(report) = declared_report(report_val, declared) else {
            tracing::warn!(
                "Ignoring a test report from step {} that the step does not declare",
                step_id
            );
            continue;
        };

        if report_val
            .get("is_claim")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            // One archive per declared report, however many entries name it
            if uploaded.insert(report.name.as_str()) {
                process_claim_report(report, tx, run_id, step_id, pool).await?;
            }
        } else if let Some(content) = report_val.get("content").and_then(|v| v.as_str()) {
            inline_bytes = inline_bytes.saturating_add(content.len());
            if inline_bytes > MAX_INLINE_REPORT_BYTES {
                tracing::warn!(
                    "Ignoring inline test report '{}' from step {}: more than {} bytes of reports in one event",
                    report.name,
                    step_id,
                    MAX_INLINE_REPORT_BYTES
                );
                continue;
            }
            let file_name = report_val
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            process_legacy_report(content, file_name, report, tx, run_id, step_id).await?;
        }
    }
    Ok(())
}

/// True when a step event names the run its step actually belongs to.
///
/// Step events arrive over NATS from runners. Without this check an event for one run
/// could carry a step id from another, and the handler would fail or complete a step
/// (and with it a run) the sender has nothing to do with.
pub(crate) fn step_belongs_to_run(
    instance: &stormchaser_model::step::StepInstance,
    run_id: stormchaser_model::RunId,
) -> bool {
    if instance.run_id == run_id {
        return true;
    }
    tracing::warn!(
        "Rejecting step event: step {} belongs to run {}, not run {}",
        instance.id,
        instance.run_id,
        run_id
    );
    false
}

/// True when the sender of a completion or failure event is allowed to finish the step.
///
/// Steps the engine handles itself (approvals, waits, built-in steps) keep a secret in
/// the database that only the engine and API can read; the event must carry it. A step
/// handed to a runner has no secret, and the runner's report is accepted as before.
pub(crate) async fn completion_authorized(
    tx: &mut Transaction<'_, Postgres>,
    instance: &stormchaser_model::step::StepInstance,
    presented: Option<&str>,
) -> bool {
    let Some(expected) = crate::db::step_completion_token(&mut **tx, instance.id).await else {
        return true;
    };
    if presented == Some(expected.as_str()) {
        return true;
    }
    tracing::warn!(
        "Rejecting event for step {} ({}): it is completed by the engine and the event carries no valid completion token",
        instance.id,
        instance.step_type
    );
    false
}

/// Whether a result for a step that is with a runner comes from the runner it is with.
///
/// Every runner can publish a step's result. Without this check any of them could
/// complete or fail a step that another is running. A step the engine finishes itself
/// is governed by its completion token instead (see [`completion_authorized`]) and is
/// not looked at here.
///
/// The runner's name is taken from the event, so this stops a runner that does not know
/// whose step it is; it does not stop one that knows and lies. Telling runners apart
/// for certain needs a NATS identity per runner.
pub(crate) async fn reporter_is_assigned_runner(
    tx: &mut Transaction<'_, Postgres>,
    instance: &stormchaser_model::step::StepInstance,
    reporter: Option<&str>,
) -> bool {
    // Engine-owned: the token has already been checked
    if crate::db::step_completion_token(&mut **tx, instance.id)
        .await
        .is_some()
    {
        return true;
    }
    let Some(assigned) = instance.runner_id.as_deref() else {
        // No runner has reported on it yet; the first to do so is the one that has it,
        // provided it is a runner. The router has checked the signature of any runner
        // that has a key; one with no key, or no name, could be anyone.
        let Some(reporter) = reporter else {
            tracing::warn!("Rejecting result for step {}: no runner named", instance.id);
            return false;
        };
        let has_key = matches!(
            crate::db::get_runner_session_key(&mut **tx, reporter).await,
            Ok(Some(_))
        );
        if !has_key && stormchaser_model::runner_auth::signatures_required() {
            tracing::warn!(
                "Rejecting result for step {} from {}: not a runner with a key",
                instance.id,
                reporter
            );
            return false;
        }
        return true;
    };
    if reporter == Some(assigned) {
        return true;
    }
    tracing::warn!(
        "Rejecting result for step {} from runner {}: the step is assigned to runner {}",
        instance.id,
        reporter.unwrap_or("<unnamed>"),
        assigned
    );
    false
}

/// Whether `reporter` may move a step that is with a runner along: the runner it is
/// assigned to, or, while none is, any runner that names itself. The router has already
/// checked that a runner with a key signed the message.
pub(crate) fn reporter_holds_step(
    instance: &stormchaser_model::step::StepInstance,
    reporter: Option<&str>,
) -> bool {
    let held = match (instance.runner_id.as_deref(), reporter) {
        (Some(assigned), Some(reporter)) => assigned == reporter,
        (None, Some(_)) => true,
        (_, None) => false,
    };
    if !held {
        tracing::warn!(
            "Ignoring a report on step {} from runner {}: the step is with runner {}",
            instance.id,
            reporter.unwrap_or("<unnamed>"),
            instance.runner_id.as_deref().unwrap_or("<none>")
        );
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use serde_json::json;

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *content).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn report(name: &str) -> TestReport {
        TestReport {
            name: name.to_string(),
            path: "out/*.xml".to_string(),
            format: "junit".to_string(),
        }
    }

    #[test]
    fn unpacks_every_file_of_a_report_archive() {
        let data = archive(&[("a.xml", b"<a/>"), ("b.xml", b"<b/>")]);
        assert_eq!(unpack_report_archive(&data).unwrap(), vec!["<a/>", "<b/>"]);
    }

    #[test]
    fn strips_nul_from_report_text() {
        let data = archive(&[("a.xml", b"<a>\0</a>")]);
        assert_eq!(unpack_report_archive(&data).unwrap(), vec!["<a></a>"]);
    }

    #[test]
    fn refuses_an_archive_that_unpacks_past_the_limit() {
        // Compresses to a few kilobytes
        let big = vec![b'a'; MAX_REPORT_UNPACKED_BYTES as usize + 1];
        let data = archive(&[("bomb.xml", &big)]);
        assert!(data.len() < 1024 * 1024);
        let err = unpack_report_archive(&data).unwrap_err();
        assert!(err.to_string().contains("unpacks to more than"), "{err}");
    }

    #[test]
    fn counts_the_unpacked_limit_across_files() {
        let half = vec![b'a'; MAX_REPORT_UNPACKED_BYTES as usize / 2 + 1];
        let data = archive(&[("one.xml", &half), ("two.xml", &half)]);
        assert!(unpack_report_archive(&data).is_err());
    }

    #[test]
    fn refuses_an_archive_with_too_many_files() {
        let files: Vec<(String, &[u8])> = (0..=MAX_REPORT_FILES)
            .map(|i| (format!("{i}.xml"), b"x".as_slice()))
            .collect();
        let files: Vec<(&str, &[u8])> = files.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        let err = unpack_report_archive(&archive(&files)).unwrap_err();
        assert!(err.to_string().contains("more than"), "{err}");
    }

    #[test]
    fn refuses_data_that_is_not_an_archive() {
        assert!(unpack_report_archive(b"not a tarball").is_err());
    }

    #[test]
    fn accepts_only_reports_the_step_declares() {
        let declared = [report("unit")];
        assert!(declared_report(&json!({"name": "unit"}), &declared).is_some());
        assert!(declared_report(&json!({"name": "other"}), &declared).is_none());
        assert!(declared_report(&json!({"is_claim": true}), &declared).is_none());
        assert!(declared_report(&json!({"name": "unit"}), &[]).is_none());
    }

    #[test]
    fn refuses_a_report_name_that_climbs_out_of_its_prefix() {
        let declared = [report("../../other-run/step/unit")];
        assert!(
            declared_report(&json!({"name": "../../other-run/step/unit"}), &declared).is_none()
        );
    }
}
