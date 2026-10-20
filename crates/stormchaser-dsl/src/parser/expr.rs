use anyhow::{anyhow, Result};
use hcl::{Block, Expression};
use serde_json::{json, Map, Value};

pub fn expr_to_string(expr: &Expression) -> Result<String> {
    match expr_to_value(expr)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        other => Ok(other.to_string()),
    }
}

pub fn expr_to_string_vec(expr: &Expression) -> Result<Vec<String>> {
    match expr {
        Expression::Array(arr) => arr.iter().map(expr_to_string).collect(),
        other => anyhow::bail!("expected a list of strings, not {}", other),
    }
}

// The parser refuses what it does not understand. An attribute or block it skipped, or
// a value of the wrong type it dropped, would look as if it had worked: `retry` and
// `allow_failure` were documented and accepted for months before anything read them.

/// The error for an attribute `context` does not define.
pub fn unknown_attribute(context: &str, key: &str) -> anyhow::Error {
    anyhow!("{} has no attribute '{}'", context, key)
}

/// The error for a block `context` does not define.
pub fn unknown_block(context: &str, identifier: &str) -> anyhow::Error {
    anyhow!("{} has no block '{}'", context, identifier)
}

/// A boolean attribute; anything else is refused.
pub fn expr_to_bool(name: &str, expr: &Expression) -> Result<bool> {
    match expr_to_value(expr)? {
        Value::Bool(b) => Ok(b),
        other => anyhow::bail!("'{}' must be true or false, not {}", name, other),
    }
}

/// A whole, non-negative number that fits in 32 bits; anything else is refused.
pub fn expr_to_u32(name: &str, expr: &Expression) -> Result<u32> {
    match expr_to_value(expr)? {
        Value::Number(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| anyhow!("'{}' must be a whole number, not {}", name, n)),
        other => anyhow::bail!("'{}' must be a whole number, not {}", name, other),
    }
}

/// A list of strings; anything else, including a list holding other values, is refused.
pub fn expr_to_strict_string_list(name: &str, expr: &Expression) -> Result<Vec<String>> {
    match expr_to_value(expr)? {
        Value::Array(items) => items
            .into_iter()
            .map(|item| match item {
                Value::String(s) => Ok(s),
                other => anyhow::bail!("'{}' must hold only strings, not {}", name, other),
            })
            .collect(),
        other => anyhow::bail!("'{}' must be a list of strings, not {}", name, other),
    }
}

pub fn expr_to_value(expr: &Expression) -> Result<Value> {
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

pub fn block_to_value(block: &Block) -> Result<Value> {
    let mut map = Map::new();
    for attr in block.body().attributes() {
        map.insert(attr.key().to_string(), expr_to_value(attr.expr())?);
    }
    for inner in block.body().blocks() {
        map.insert(inner.identifier().to_string(), block_to_value(inner)?);
    }
    Ok(Value::Object(map))
}

/// Adds a `spec = ...` attribute to a step's spec. `spec = { ... }` means the same as a
/// `spec { ... }` block; any other value, such as an expression resolved when the step
/// runs, is kept under `spec`, where the engine also looks.
pub fn merge_spec_attr(spec_map: &mut Map<String, Value>, expr: &Expression) -> Result<()> {
    match expr_to_value(expr)? {
        Value::Object(fields) => spec_map.extend(fields),
        other => {
            spec_map.insert("spec".to_string(), other);
        }
    }
    Ok(())
}

/// A step's or step library's param: a string, or a number or boolean as its text.
/// Params are strings once resolved; a list or a map has no single text to be.
pub fn expr_to_param(name: &str, expr: &Expression) -> Result<String> {
    match expr_to_value(expr)? {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        other => anyhow::bail!(
            "Param '{}' must be a string, number or boolean, not {}",
            name,
            other
        ),
    }
}
