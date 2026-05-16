use anyhow::Result;
use chrono::{DateTime, Utc};
use stormchaser_dsl::ast::Step;
use stormchaser_model::{LogBackend, StepInstanceId};

pub async fn scrape_outputs_from_logs(
    tx: &mut sqlx::PgConnection,
    step_id: StepInstanceId,
    dsl_step: &Step,
    backend: &LogBackend,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
) -> Result<()> {
    if dsl_step.outputs.is_empty() {
        return Ok(());
    }

    tracing::info!("Scraping outputs from logs for step {}", dsl_step.name);
    let logs = backend
        .fetch_step_logs(
            &dsl_step.name,
            step_id,
            started_at,
            finished_at,
            Some(5000), // Get up to 5000 lines for output scraping
        )
        .await
        .unwrap_or_default();

    let mut filtered_logs = Vec::new();
    let mut in_output_block = dsl_step.start_marker.is_none();

    for line in &logs {
        if let Some(start) = &dsl_step.start_marker {
            if line.contains(start) {
                in_output_block = true;
                continue;
            }
        }
        if let Some(end) = &dsl_step.end_marker {
            if line.contains(end) {
                in_output_block = false;
                continue;
            }
        }
        if in_output_block {
            filtered_logs.push(line);
        }
    }

    for extraction in &dsl_step.outputs {
        if extraction.source == "logs" || extraction.source == "stdout" {
            if let Some(regex_str) = &extraction.regex {
                if let Ok(re) = regex::Regex::new(regex_str) {
                    for line in filtered_logs.iter().rev() {
                        if let Some(caps) = re.captures(line) {
                            let value = if let Some(marker) = &extraction.marker {
                                if line.contains(marker) {
                                    caps.get(extraction.group.unwrap_or(1) as usize)
                                        .map(|m| m.as_str().to_string())
                                } else {
                                    None
                                }
                            } else {
                                caps.get(extraction.group.unwrap_or(1) as usize)
                                    .map(|m| m.as_str().to_string())
                            };

                            if let Some(val) = value {
                                let final_val = if extraction.format.as_deref() == Some("json") {
                                    serde_json::from_str(&val).unwrap_or(serde_json::json!(val))
                                } else {
                                    serde_json::json!(val)
                                };

                                crate::db::upsert_step_output_with_sensitivity(
                                    &mut *tx,
                                    step_id,
                                    &extraction.name,
                                    &final_val,
                                    extraction.sensitive.unwrap_or(false),
                                )
                                .await?;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
