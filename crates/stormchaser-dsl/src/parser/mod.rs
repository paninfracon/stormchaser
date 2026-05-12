use anyhow::{Context, Result};
use hcl::Body;

use crate::ast::{Step, Workflow};

pub mod expr;
pub mod provision;
pub mod step;
pub mod workflow;

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
                dsl_version = expr::expr_to_string(attribute.expr())?;
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
                description = Some(expr::expr_to_string(attribute.expr())?);
            } else if attribute.key() == "cron" {
                cron = Some(expr::expr_to_string(attribute.expr())?);
            }
        }

        let blocks = workflow::parse_workflow_blocks(workflow_block.body())?;

        Ok(Workflow {
            is_template,
            dsl_version,
            name,
            description,
            cron,
            libraries: blocks.libraries,
            step_libraries: blocks.step_libraries,
            includes: blocks.includes,
            strategy: blocks.strategy,
            quotas: blocks.quotas,
            aliases: blocks.aliases,
            storage: blocks.storage,
            inputs: blocks.inputs,
            queries: blocks.queries,
            inputs_schema: blocks.inputs_schema,
            inputs_view: blocks.inputs_view,
            outputs: blocks.outputs,
            handlers: blocks.handlers,
            steps: blocks.steps,
            on_failure: blocks.on_failure,
            finally: blocks.finally,
        })
    }

    pub fn parse_steps(&self, body: &Body) -> Result<Vec<Step>> {
        step::parse_steps(body)
    }
}
