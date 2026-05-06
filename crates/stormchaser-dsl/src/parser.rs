use anyhow::{Context, Result};
use hcl::{Block, Body, Expression};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::ast::{Step, Workflow};
use stormchaser_model::dsl;

type ParsedWorkflowBlocks = (
    Vec<Step>,
    Vec<dsl::Storage>,
    Vec<dsl::Input>,
    Vec<dsl::Output>,
    Vec<dsl::StepLibrary>,
    Vec<dsl::Include>,
);

/// A parser for translating Stormchaser HCL DSL into an executable Workflow model.
pub struct StormchaserParser;

impl Default for StormchaserParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StormchaserParser {
    /// Creates a new instance of the `StormchaserParser`.
    pub fn new() -> Self {
        Self
    }

    /// Parses the provided HCL DSL string and returns a `Workflow` instance.
    pub fn parse(&self, dsl: &str) -> Result<Workflow> {
        let body: Body = hcl::from_str(dsl)?;

        let mut dsl_version = String::new();
        for attribute in body.attributes() {
            if attribute.key() == "stormchaser_dsl_version" {
                dsl_version = expr_to_string(attribute.expr())?;
            }
        }

        let workflow_block = body
            .blocks()
            .find(|b| b.identifier() == "workflow" || b.identifier() == "workflow_template")
            .context("Missing 'workflow' or 'workflow_template' block")?;

        let is_template = workflow_block.identifier() == "workflow_template";

        let name = workflow_block
            .labels()
            .first()
            .map(|l| l.as_str().to_string())
            .context("Workflow block must have a name label")?;

        let mut description = None;
        let mut cron = None;

        for attribute in workflow_block.body().attributes() {
            if attribute.key() == "description" {
                description = Some(expr_to_string(attribute.expr())?);
            } else if attribute.key() == "cron" {
                cron = Some(expr_to_string(attribute.expr())?);
            }
        }

        let (steps, storage, inputs, outputs, step_libraries, includes) =
            self.parse_workflow_blocks(workflow_block.body())?;

        Ok(Workflow {
            is_template,
            dsl_version,
            name,
            description,
            cron,
            libraries: vec![],
            step_libraries,
            includes,
            strategy: None,
            quotas: None,
            storage,
            inputs,
            outputs,
            handlers: vec![],
            steps,
            on_failure: None,
            finally: None,
        })
    }

    fn parse_workflow_blocks(&self, body: &Body) -> Result<ParsedWorkflowBlocks> {
        let mut steps = Vec::new();
        let mut storage = Vec::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut step_libraries = Vec::new();
        let mut includes = Vec::new();

        for block in body.blocks() {
            match block.identifier() {
                "steps" => {
                    steps = self.parse_steps(block.body())?;
                }
                "storage" => {
                    storage.push(self.parse_storage_block(block)?);
                }
                "input" => {
                    inputs.push(self.parse_input_block(block)?);
                }
                "output" => {
                    outputs.push(self.parse_output_block(block)?);
                }
                "step_library" => {
                    step_libraries.push(self.parse_step_library_block(block)?);
                }
                "include" => {
                    includes.push(self.parse_include_block(block)?);
                }
                _ => {}
            }
        }

        Ok((steps, storage, inputs, outputs, step_libraries, includes))
    }

    fn parse_storage_block(&self, block: &Block) -> Result<dsl::Storage> {
        let name = block
            .labels()
            .first()
            .map(|l| l.as_str().to_string())
            .context("Storage block must have a name label")?;
        let mut size = "1Gi".to_string();
        let mut backend = None;
        let mut provision = Vec::new();
        let mut artifacts = Vec::new();

        for attr in block.body().attributes() {
            if attr.key() == "size" {
                size = expr_to_string(attr.expr())?;
            } else if attr.key() == "backend" {
                backend = Some(expr_to_string(attr.expr())?);
            }
        }

        for inner in block.body().blocks() {
            if inner.identifier() == "artifact" {
                let art_name = inner
                    .labels()
                    .first()
                    .map(|l| l.as_str().to_string())
                    .context("Artifact block must have a name label")?;
                let mut path = String::new();
                let mut retention = "on_success".to_string();
                for attr in inner.body().attributes() {
                    if attr.key() == "path" {
                        path = expr_to_string(attr.expr())?;
                    } else if attr.key() == "retention" {
                        retention = expr_to_string(attr.expr())?;
                    }
                }
                artifacts.push(dsl::Artifact {
                    name: art_name,
                    path,
                    retention,
                });
            } else if inner.identifier() == "provision" {
                if inner.labels().is_empty() {
                    for prov_block in inner.body().blocks() {
                        provision.push(parse_provision_sub_block(prov_block)?);
                    }
                } else {
                    provision.push(parse_provision_legacy_block(inner)?);
                }
            }
        }

        Ok(dsl::Storage {
            name,
            backend,
            size,
            provision,
            preserve: vec![],
            artifacts,
            retainment: None,
        })
    }

    fn parse_input_block(&self, block: &Block) -> Result<dsl::Input> {
        let name = block
            .labels()
            .first()
            .map(|l| l.as_str().to_string())
            .context("Input block must have a name label")?;
        let mut r#type = "string".to_string();
        let mut default = None;
        for attr in block.body().attributes() {
            if attr.key() == "type" {
                r#type = expr_to_string(attr.expr())?;
            } else if attr.key() == "default" {
                default = Some(expr_to_value(attr.expr())?);
            }
        }
        Ok(dsl::Input {
            name,
            r#type,
            description: None,
            default,
            validation: None,
            options: None,
            query: None,
        })
    }

    fn parse_output_block(&self, block: &Block) -> Result<dsl::Output> {
        let name = block
            .labels()
            .first()
            .map(|l| l.as_str().to_string())
            .context("Output block must have a name label")?;
        let mut value = String::new();
        for attr in block.body().attributes() {
            if attr.key() == "value" {
                value = expr_to_string(attr.expr())?;
            }
        }
        Ok(dsl::Output { name, value })
    }

    fn parse_step_library_block(&self, block: &Block) -> Result<dsl::StepLibrary> {
        let name = block
            .labels()
            .first()
            .map(|l| l.as_str().to_string())
            .context("step_library block must have a name label")?;
        let mut r#type = String::new();
        let mut params = HashMap::new();
        let mut spec_map = Map::new();
        let mut timeout = None;
        let mut allow_failure = None;

        for attr in block.body().attributes() {
            match attr.key() {
                "type" => r#type = expr_to_string(attr.expr())?,
                "params" => {
                    if let Value::Object(obj) = expr_to_value(attr.expr())? {
                        for (k, v) in obj {
                            if let Value::String(s) = v {
                                params.insert(k, s);
                            }
                        }
                    }
                }
                "timeout" => timeout = Some(expr_to_string(attr.expr())?),
                "allow_failure" => {
                    if let Value::Bool(b) = expr_to_value(attr.expr())? {
                        allow_failure = Some(b);
                    }
                }
                _ => {}
            }
        }

        for inner in block.body().blocks() {
            if inner.identifier() == "spec" {
                for attr in inner.body().attributes() {
                    spec_map.insert(attr.key().to_string(), expr_to_value(attr.expr())?);
                }
                for inner_block in inner.body().blocks() {
                    let key = inner_block.identifier().to_string();
                    let value = block_to_value(inner_block)?;
                    spec_map.insert(key, value);
                }
            } else if inner.identifier() == "params" {
                for attr in inner.body().attributes() {
                    params.insert(attr.key().to_string(), expr_to_string(attr.expr())?);
                }
            }
        }

        Ok(dsl::StepLibrary {
            name,
            r#type,
            params,
            spec: Value::Object(spec_map),
            timeout,
            allow_failure,
            retry: None,
        })
    }

    fn parse_include_block(&self, block: &Block) -> Result<dsl::Include> {
        let name = block
            .labels()
            .first()
            .map(|l| l.as_str().to_string())
            .context("include block must have a name label")?;
        let mut workflow = String::new();
        let mut inputs_map = HashMap::new();

        for attr in block.body().attributes() {
            if attr.key() == "workflow" {
                workflow = expr_to_string(attr.expr())?;
            } else if attr.key() == "inputs" {
                if let Value::Object(obj) = expr_to_value(attr.expr())? {
                    for (k, v) in obj {
                        if let Value::String(s) = v {
                            inputs_map.insert(k, s);
                        } else {
                            inputs_map.insert(k, v.to_string());
                        }
                    }
                }
            }
        }

        Ok(dsl::Include {
            name,
            workflow,
            inputs: inputs_map,
        })
    }

    fn parse_step_strategy_block(&self, inner_block: &Block) -> Result<dsl::Strategy> {
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

    fn parse_step_reports_block(&self, inner_block: &Block) -> Result<Vec<dsl::TestReport>> {
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
        &self,
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
                    format: None,
                    regex,
                    group,
                    sensitive,
                });
            }
        }
        Ok((start_marker, end_marker, outputs))
    }

    pub fn parse_steps(&self, body: &Body) -> Result<Vec<Step>> {
        let mut steps = Vec::new();
        for block in body.blocks() {
            if block.identifier() == "step" {
                steps.push(self.parse_single_step(block)?);
            }
        }
        Ok(steps)
    }

    fn parse_single_step(&self, block: &Block) -> Result<Step> {
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

        self.apply_step_attributes(block, &mut step, &mut spec_map)?;
        self.apply_step_inner_blocks(block, &mut step, &mut spec_map)?;

        step.spec = Value::Object(spec_map);
        Ok(step)
    }

    fn apply_step_attributes(
        &self,
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
        &self,
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
                "steps" => {
                    step.steps = Some(self.parse_steps(inner_block.body())?);
                }
                "strategy" => {
                    step.strategy = Some(self.parse_step_strategy_block(inner_block)?);
                }
                "reports" => {
                    step.reports
                        .extend(self.parse_step_reports_block(inner_block)?);
                }
                "outputs" => {
                    let (sm, em, out) = self.parse_step_outputs_block(inner_block)?;
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
}

/// Parses a provision sub-block in the new syntax:
/// `provision { <resource_type> "name" { ... } }`
fn parse_provision_sub_block(prov_block: &Block) -> Result<dsl::Provision> {
    let resource_type = prov_block.identifier().to_string();
    let name = prov_block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Provision sub-block must have a name label")?;
    parse_provision_attributes(name, resource_type, prov_block.body())
}

/// Parses a provision block in the legacy syntax:
/// `provision "name" { resource_type = "download" ... }`
fn parse_provision_legacy_block(block: &Block) -> Result<dsl::Provision> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Legacy provision block must have a name label")?;
    let mut resource_type = "download".to_string();
    for attr in block.body().attributes() {
        if attr.key() == "resource_type" {
            resource_type = expr_to_string(attr.expr())?;
        }
    }
    parse_provision_attributes(name, resource_type, block.body())
}

/// Extracts the common provision fields from a block body, for both syntaxes.
fn parse_provision_attributes(
    name: String,
    resource_type: String,
    body: &Body,
) -> Result<dsl::Provision> {
    let mut source = None;
    let mut url = None;
    let mut destination = "/".to_string();
    let mut mode = None;
    let mut checksum = None;
    let mut from = None;

    for attr in body.attributes() {
        match attr.key() {
            "source" => source = Some(expr_to_string(attr.expr())?),
            "url" => url = Some(expr_to_string(attr.expr())?),
            "destination" => destination = expr_to_string(attr.expr())?,
            "mode" => mode = Some(expr_to_string(attr.expr())?),
            "checksum" => checksum = Some(expr_to_string(attr.expr())?),
            "from" => from = Some(expr_to_string(attr.expr())?),
            _ => {}
        }
    }

    Ok(dsl::Provision {
        name,
        resource_type,
        source,
        url,
        destination,
        mode,
        checksum,
        from,
    })
}

fn expr_to_string(expr: &Expression) -> Result<String> {
    match expr_to_value(expr)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        other => Ok(other.to_string()),
    }
}

fn expr_to_string_vec(expr: &Expression) -> Result<Vec<String>> {
    match expr {
        Expression::Array(arr) => {
            let mut result = Vec::new();
            for item in arr {
                if let Expression::String(s) = item {
                    result.push(s.clone());
                } else if let Ok(s) = expr_to_string(item) {
                    result.push(s);
                }
            }
            Ok(result)
        }
        _ => Ok(vec![]),
    }
}

fn expr_to_value(expr: &Expression) -> Result<Value> {
    match expr {
        Expression::String(s) => Ok(Value::String(s.clone())),
        Expression::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(json!(i))
            } else if let Some(u) = n.as_u64() {
                Ok(json!(u))
            } else if let Some(f) = n.as_f64() {
                Ok(json!(f))
            } else {
                Ok(json!(0))
            }
        }
        Expression::Bool(b) => Ok(Value::Bool(*b)),
        Expression::Null => Ok(Value::Null),
        Expression::Array(arr) => {
            let mut vals = Vec::new();
            for e in arr {
                vals.push(expr_to_value(e)?);
            }
            Ok(Value::Array(vals))
        }
        Expression::Object(obj) => {
            let mut map = Map::new();
            for (k, v) in obj {
                map.insert(k.to_string(), expr_to_value(v)?);
            }
            Ok(Value::Object(map))
        }
        Expression::Traversal(_) => {
            // Convert HCL traversal (e.g. inputs.repo_url) to ${inputs.repo_url}
            Ok(Value::String(format!("${{{}}}", expr)))
        }
        Expression::Variable(_) => {
            // Convert HCL variable (e.g. var_name) to ${var_name}
            Ok(Value::String(format!("${{{}}}", expr)))
        }
        Expression::TemplateExpr(t) => {
            let s = t.to_string();
            if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
                // Strip outer quotes from quoted template
                Ok(Value::String(s[1..s.len() - 1].to_string()))
            } else if s.starts_with("<<") {
                // Heuristic for heredoc: find first newline and last newline
                let lines: Vec<&str> = s.lines().collect();
                if lines.len() >= 2 {
                    let content = lines[1..lines.len() - 1].join("\n");
                    Ok(Value::String(content.trim().to_string()))
                } else {
                    Ok(Value::String(s))
                }
            } else {
                Ok(Value::String(s))
            }
        }
        _ => {
            // Fallback for more complex expressions like function calls if needed.
            let expr_str = expr.to_string();
            Ok(serde_json::from_str(&expr_str).unwrap_or(Value::String(expr_str)))
        }
    }
}

fn block_to_value(block: &Block) -> Result<Value> {
    let mut map = Map::new();
    for attr in block.body().attributes() {
        map.insert(attr.key().to_string(), expr_to_value(attr.expr())?);
    }
    for inner in block.body().blocks() {
        map.insert(inner.identifier().to_string(), block_to_value(inner)?);
    }
    Ok(Value::Object(map))
}
