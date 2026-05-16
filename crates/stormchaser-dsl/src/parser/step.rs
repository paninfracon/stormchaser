use std::collections::HashMap;

use anyhow::{Context, Result};
use hcl::{Block, Body};
use serde_json::{Map, Value};
use stormchaser_model::dsl;

use crate::ast::Step;

use super::expr::{block_to_value, expr_to_string, expr_to_string_vec, expr_to_value};

pub fn parse_steps(body: &Body) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for block in body.blocks() {
        if block.identifier() == "step" {
            steps.push(parse_single_step(block)?);
        }
    }
    Ok(steps)
}

fn parse_single_step(block: &Block) -> Result<Step> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Step block must have a name label")?;
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
            "next" => step.next = expr_to_string_vec(attr.expr())?,
            "iterate" => step.iterate = Some(expr_to_string(attr.expr())?),
            "iterate_as" | "as" => step.iterate_as = Some(expr_to_string(attr.expr())?),
            "allow_failure" => {
                if let Value::Bool(b) = expr_to_value(attr.expr())? {
                    step.allow_failure = Some(b);
                }
            }
            "timeout" => step.timeout = Some(expr_to_string(attr.expr())?),
            "artifacts" => step.artifacts = Some(expr_to_string_vec(attr.expr())?),
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
                    step.params
                        .insert(attr.key().to_string(), expr_to_string(attr.expr())?);
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
                step.strategy = Some(parse_step_strategy_block(inner_block)?);
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
            "fail_fast" => {
                if let Value::Bool(b) = expr_to_value(attr.expr())? {
                    s.fail_fast = Some(b);
                }
            }
            "max_parallel" => {
                if let Value::Number(n) = expr_to_value(attr.expr())? {
                    s.max_parallel = n.as_u64().map(|v| v as u32);
                }
            }
            _ => {}
        }
    }
    Ok(s)
}

fn parse_step_reports_block(inner_block: &Block) -> Result<Vec<dsl::TestReport>> {
    let mut reports = Vec::new();
    for report_block in inner_block.body().blocks() {
        if report_block.identifier() == "report" {
            let art_name = report_block
                .labels()
                .first()
                .map(|l| l.as_str().to_string())
                .context("Report block must have a name label")?;

            let mut path = String::new();
            let mut format = "junit".to_string();
            for attr in report_block.body().attributes() {
                if attr.key() == "path" {
                    path = expr_to_string(attr.expr())?;
                } else if attr.key() == "format" {
                    format = expr_to_string(attr.expr())?;
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
        if attr.key() == "start_marker" {
            start_marker = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "end_marker" {
            end_marker = Some(expr_to_string(attr.expr())?);
        }
    }
    for output_block in inner_block.body().blocks() {
        if output_block.identifier() == "output" {
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
                    "group" => {
                        if let Value::Number(n) = expr_to_value(attr.expr())? {
                            group = n.as_u64().map(|v| v as u32);
                        }
                    }
                    "sensitive" => {
                        if let Value::Bool(b) = expr_to_value(attr.expr())? {
                            sensitive = Some(b);
                        }
                    }
                    _ => {}
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
