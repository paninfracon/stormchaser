//! Step libraries: reusable step definitions that a step names as its type.
//!
//! A step whose type is a library's name becomes a step of the library's type, with
//! the library's spec, params, timeout, failure policy and retry policy, each
//! overridden by what the step sets itself. The spec is merged one key deep: a key the
//! step sets replaces the library's value for it whole.
//!
//! Params are the library's arguments. A spec refers to them as `${params.<name>}`; the
//! engine resolves the params first, against the run's inputs, secrets and step
//! outputs, then the spec, with the params available to it.

use anyhow::{bail, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::LazyLock;
use stormchaser_model::dsl::{Step, StepLibrary, Workflow};

/// Applies the workflow's step libraries to every step that names one, and checks
/// that each param their specs use is set.
pub fn apply_step_libraries(workflow: &mut Workflow) -> Result<()> {
    let libraries = workflow.step_libraries.clone();
    let groups = std::iter::once(&mut workflow.steps)
        .chain(workflow.on_failure.as_mut())
        .chain(workflow.finally.as_mut());
    for steps in groups {
        for step in steps.iter_mut() {
            apply_recursively(step, &libraries)?;
        }
    }
    Ok(())
}

fn apply_recursively(step: &mut Step, libraries: &[StepLibrary]) -> Result<()> {
    apply_step_library(step, libraries)?;
    for inner in step.steps.iter_mut().flatten() {
        apply_recursively(inner, libraries)?;
    }
    if let Some(on_failure) = step.on_failure.as_deref_mut() {
        apply_recursively(on_failure, libraries)?;
    }
    Ok(())
}

/// Applies the step library `step` names as its type, if there is one, and checks that
/// each param its spec uses is set. A step that names no library is checked only when
/// it sets params of its own: its type may be a library a later include brings.
pub fn apply_step_library(step: &mut Step, libraries: &[StepLibrary]) -> Result<()> {
    let library = libraries.iter().find(|l| l.name == step.r#type);
    if let Some(library) = library {
        step.r#type = library.r#type.clone();
        step.spec = match (&library.spec, std::mem::take(&mut step.spec)) {
            (Value::Object(base), Value::Object(own)) => {
                let mut merged = base.clone();
                merged.extend(own);
                Value::Object(merged)
            }
            (base, Value::Null) => base.clone(),
            (_, own) => own,
        };
        let mut params = library.params.clone();
        params.extend(std::mem::take(&mut step.params));
        step.params = params;
        step.timeout = step.timeout.take().or_else(|| library.timeout.clone());
        step.allow_failure = step.allow_failure.or(library.allow_failure);
        step.retry = step.retry.take().or_else(|| library.retry.clone());
    } else if step.params.is_empty() {
        return Ok(());
    }

    for name in param_references(&step.spec) {
        if !step.params.contains_key(&name) {
            match library {
                Some(library) => bail!(
                    "Step '{}' uses params.{}, which neither it nor its step library '{}' sets",
                    step.name,
                    name,
                    library.name
                ),
                None => bail!(
                    "Step '{}' uses params.{}, which it does not set",
                    step.name,
                    name
                ),
            }
        }
    }
    Ok(())
}

/// The params that the `${...}` expressions in `spec` use as `params.<name>` or
/// `params["<name>"]`. `$${...}` is a literal `${...}`, not an expression.
fn param_references(spec: &Value) -> BTreeSet<String> {
    static EXPRESSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(^|[^$])\$\{([^}]*)\}").unwrap());
    static PARAM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\bparams(?:\.([A-Za-z_][A-Za-z0-9_-]*)|\["([^"]+)"\])"#).unwrap()
    });

    let mut names = BTreeSet::new();
    let mut strings = vec![spec];
    while let Some(value) = strings.pop() {
        match value {
            Value::String(text) => {
                for expression in EXPRESSION.captures_iter(text) {
                    for param in PARAM.captures_iter(&expression[2]) {
                        let name = param.get(1).or_else(|| param.get(2)).unwrap();
                        names.insert(name.as_str().to_string());
                    }
                }
            }
            Value::Array(items) => strings.extend(items),
            Value::Object(fields) => strings.extend(fields.values()),
            _ => {}
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::param_references;
    use serde_json::json;

    #[test]
    fn param_references_are_found_in_expressions_only() {
        let spec = json!({
            "image": "python:${params.version}-slim",
            "command": ["sh", "-c", "echo ${params[\"greeting\"]} $${params.literal}"],
            "env": [{ "name": "X", "value": "params.not_an_expression" }],
        });
        let names: Vec<String> = param_references(&spec).into_iter().collect();
        assert_eq!(names, ["greeting", "version"]);
    }
}
