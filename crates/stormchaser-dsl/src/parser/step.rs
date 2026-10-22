use std::collections::HashMap;

use anyhow::{Context, Result};
use hcl::{Block, Body};
use serde_json::{Map, Value};
use stormchaser_model::dsl;

use crate::ast::Step;

use super::expr::{
    block_to_value, expr_to_bool, expr_to_param, expr_to_string, expr_to_string_vec, expr_to_u32,
    expr_to_value, merge_spec_attr, unknown_attribute, unknown_block,
};

pub fn parse_steps(body: &Body) -> Result<Vec<Step>> {
    if let Some(attr) = body.attributes().next() {
        return Err(unknown_attribute("A list of steps", attr.key()));
    }
    let mut steps = Vec::new();
    for block in body.blocks() {
        if block.identifier() != "step" {
            return Err(unknown_block("A list of steps", block.identifier()));
        }
        steps.push(parse_single_step(block)?);
    }
    Ok(steps)
}

fn parse_single_step(block: &Block) -> Result<Step> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Step block must have a name label")?;
    // The name ends up in container and job names, labels and log queries
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!(
            "Step name '{}' is invalid: use only letters, digits, '_' and '-'",
            name
        );
    }
    let r#type = block
        .labels()
        .get(1)
        .map(|l| l.as_str().to_string())
        .context("Step block must have a type label")?;

    let mut step = Step {
        name,
        r#type,
        condition: None,
        params: HashMap::new(),
        spec: Value::Null,
        strategy: None,
        aggregation: vec![],
        iterate: None,
        iterate_as: None,
        steps: None,
        next: vec![],
        on_failure: None,
        aliases: HashMap::new(),
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        outputs: vec![],
        reports: vec![],
        artifacts: None,
    };

    let mut spec_map = Map::new();

    apply_step_attributes(block, &mut step, &mut spec_map)?;
    apply_step_inner_blocks(block, &mut step, &mut spec_map)?;

    step.spec = Value::Object(spec_map);
    Ok(step)
}

fn apply_step_attributes(
    block: &Block,
    step: &mut Step,
    spec_map: &mut Map<String, Value>,
) -> Result<()> {
    for attr in block.body().attributes() {
        match attr.key() {
            "condition" => step.condition = Some(expr_to_string(attr.expr())?),
            "next" => {
                step.next = expr_to_string_vec(attr.expr())
                    .with_context(|| format!("Step '{}' next", step.name))?
            }
            "iterate" => step.iterate = Some(expr_to_string(attr.expr())?),
            "iterate_as" | "as" => step.iterate_as = Some(expr_to_string(attr.expr())?),
            "allow_failure" => {
                step.allow_failure = Some(
                    expr_to_bool("allow_failure", attr.expr())
                        .with_context(|| format!("Step '{}'", step.name))?,
                )
            }
            "timeout" => step.timeout = Some(expr_to_string(attr.expr())?),
            "artifacts" => {
                step.artifacts = Some(
                    expr_to_string_vec(attr.expr())
                        .with_context(|| format!("Step '{}' artifacts", step.name))?,
                )
            }
            "spec" => merge_spec_attr(spec_map, attr.expr())?,
            _ => {
                spec_map.insert(attr.key().to_string(), expr_to_value(attr.expr())?);
            }
        }
    }
    Ok(())
}

fn apply_step_inner_blocks(
    block: &Block,
    step: &mut Step,
    spec_map: &mut Map<String, Value>,
) -> Result<()> {
    for inner_block in block.body().blocks() {
        match inner_block.identifier() {
            "params" => {
                for attr in inner_block.body().attributes() {
                    step.params.insert(
                        attr.key().to_string(),
                        expr_to_param(attr.key(), attr.expr())?,
                    );
                }
            }
            "aliases" => {
                for attr in inner_block.body().attributes() {
                    step.aliases
                        .insert(attr.key().to_string(), expr_to_string(attr.expr())?);
                }
            }
            "steps" => {
                step.steps = Some(parse_steps(inner_block.body())?);
            }
            "strategy" => {
                step.strategy = Some(
                    parse_step_strategy_block(inner_block)
                        .with_context(|| format!("Step '{}'", step.name))?,
                );
            }
            "retry" => {
                step.retry = Some(
                    parse_retry_block(inner_block)
                        .with_context(|| format!("Step '{}'", step.name))?,
                );
            }
            "reports" => {
                step.reports.extend(parse_step_reports_block(inner_block)?);
            }
            "outputs" => {
                let (sm, em, out) = parse_step_outputs_block(inner_block)?;
                if sm.is_some() {
                    step.start_marker = sm;
                }
                if em.is_some() {
                    step.end_marker = em;
                }
                step.outputs.extend(out);
            }
            "spec" => {
                for attr in inner_block.body().attributes() {
                    spec_map.insert(attr.key().to_string(), expr_to_value(attr.expr())?);
                }
                for nested_block in inner_block.body().blocks() {
                    let key = nested_block.identifier().to_string();
                    let value = block_to_value(nested_block)?;
                    spec_map.insert(key, value);
                }
            }
            _ => {
                spec_map.insert(
                    inner_block.identifier().to_string(),
                    block_to_value(inner_block)?,
                );
            }
        }
    }
    Ok(())
}

/// Parses a `retry { count = 3  backoff = "exponential"  delay = "10s"  max_delay = "5m" }`
/// block. `count` is required; `backoff` defaults to `fixed`.
pub(super) fn parse_retry_block(block: &Block) -> Result<dsl::RetryPolicy> {
    let mut count = None;
    let mut retry = dsl::RetryPolicy {
        count: 0,
        backoff: "fixed".to_string(),
        delay: None,
        max_delay: None,
    };
    for attr in block.body().attributes() {
        match attr.key() {
            "count" => {
                count = match expr_to_value(attr.expr())? {
                    Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
                    _ => None,
                };
                if count.is_none() {
                    anyhow::bail!("retry count must be a whole number of retries");
                }
            }
            "backoff" => retry.backoff = expr_to_string(attr.expr())?,
            "delay" => retry.delay = Some(expr_to_string(attr.expr())?),
            "max_delay" => retry.max_delay = Some(expr_to_string(attr.expr())?),
            other => anyhow::bail!("retry has no attribute '{}'", other),
        }
    }
    retry.count = count.context("retry needs a count")?;
    retry.validate().map_err(anyhow::Error::msg)?;
    Ok(retry)
}

fn parse_step_strategy_block(inner_block: &Block) -> Result<dsl::Strategy> {
    let mut s = dsl::Strategy {
        affinity: None,
        fail_fast: None,
        max_parallel: None,
        process_allow_list: None,
    };
    for attr in inner_block.body().attributes() {
        match attr.key() {
            "affinity" => s.affinity = Some(expr_to_string(attr.expr())?),
            "fail_fast" => s.fail_fast = Some(expr_to_bool("fail_fast", attr.expr())?),
            "max_parallel" => s.max_parallel = Some(expr_to_u32("max_parallel", attr.expr())?),
            other => return Err(unknown_attribute("A step's strategy", other)),
        }
    }
    Ok(s)
}

fn parse_step_reports_block(inner_block: &Block) -> Result<Vec<dsl::TestReport>> {
    let mut reports = Vec::new();
    if let Some(attr) = inner_block.body().attributes().next() {
        return Err(unknown_attribute("A step's reports", attr.key()));
    }
    for report_block in inner_block.body().blocks() {
        if report_block.identifier() != "report" {
            return Err(unknown_block("A step's reports", report_block.identifier()));
        }
        {
            let art_name = report_block
                .labels()
                .first()
                .map(|l| l.as_str().to_string())
                .context("Report block must have a name label")?;

            let mut path = String::new();
            let mut format = "junit".to_string();
            for attr in report_block.body().attributes() {
                match attr.key() {
                    "path" => path = expr_to_string(attr.expr())?,
                    "format" => format = expr_to_string(attr.expr())?,
                    other => {
                        return Err(unknown_attribute(&format!("Report '{}'", art_name), other))
                    }
                }
            }
            reports.push(dsl::TestReport {
                name: art_name,
                path,
                format,
            });
        }
    }
    Ok(reports)
}

fn parse_step_outputs_block(
    inner_block: &Block,
) -> Result<(Option<String>, Option<String>, Vec<dsl::OutputExtraction>)> {
    let mut start_marker = None;
    let mut end_marker = None;
    let mut outputs = Vec::new();

    for attr in inner_block.body().attributes() {
        match attr.key() {
            "start_marker" => start_marker = Some(expr_to_string(attr.expr())?),
            "end_marker" => end_marker = Some(expr_to_string(attr.expr())?),
            other => return Err(unknown_attribute("A step's outputs", other)),
        }
    }
    for output_block in inner_block.body().blocks() {
        if output_block.identifier() != "output" {
            return Err(unknown_block("A step's outputs", output_block.identifier()));
        }
        {
            let name = output_block
                .labels()
                .first()
                .map(|l| l.as_str().to_string())
                .context("Output block must have a name label")?;

            let mut source = "logs".to_string();
            let mut marker = None;
            let mut format = None;
            let mut regex = None;
            let mut group = None;
            let mut sensitive = None;

            for attr in output_block.body().attributes() {
                match attr.key() {
                    "source" => {
                        source = expr_to_string(attr.expr())?;
                    }
                    "marker" => {
                        marker = Some(expr_to_string(attr.expr())?);
                    }
                    "format" => {
                        format = Some(expr_to_string(attr.expr())?);
                    }
                    "regex" => {
                        regex = Some(expr_to_string(attr.expr())?);
                    }
                    "group" => group = Some(expr_to_u32("group", attr.expr())?),
                    "sensitive" => sensitive = Some(expr_to_bool("sensitive", attr.expr())?),
                    other => return Err(unknown_attribute(&format!("Output '{}'", name), other)),
                }
            }

            outputs.push(dsl::OutputExtraction {
                name,
                source,
                marker,
                format,
                regex,
                group,
                sensitive,
            });
        }
    }
    Ok((start_marker, end_marker, outputs))
}
