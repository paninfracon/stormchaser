//! Hcl expression evaluation models and conversion utilities.

use anyhow::Result;
use hcl::eval::{Context as HclContext, Evaluate};
use hcl::Value as HclValue;
use regex::Regex;
use serde_json::Value;

/// Resolves HCL expressions embedded in strings via `${...}` syntax within a JSON Value.
pub fn resolve_expressions(value: &mut Value, ctx: &HclContext, strict: bool) -> Result<()> {
    match value {
        Value::String(s) => match evaluate_string(s, ctx) {
            Ok(Some(new_val)) => {
                *value = new_val;
            }
            Err(e) if strict => {
                return Err(e);
            }
            _ => {}
        },
        Value::Array(arr) => {
            for v in arr {
                resolve_expressions(v, ctx, strict)?;
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                resolve_expressions(v, ctx, strict)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Converts a JSON value to an equivalent HCL value.
pub fn json_to_hcl(v: Value) -> HclValue {
    match v {
        Value::Null => HclValue::Null,
        Value::Bool(b) => HclValue::Bool(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                HclValue::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                HclValue::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                HclValue::Number(hcl::Number::from_f64(f).unwrap_or_else(|| 0.into()))
            } else {
                HclValue::Null
            }
        }
        Value::String(s) => HclValue::String(s),
        Value::Array(arr) => {
            let list: Vec<HclValue> = arr.into_iter().map(json_to_hcl).collect();
            HclValue::Array(list)
        }
        Value::Object(map) => {
            let mut hcl_map = hcl::Map::new();
            for (k, v) in map {
                hcl_map.insert(k, json_to_hcl(v));
            }
            HclValue::Object(hcl_map)
        }
    }
}

/// Converts an HCL value back to an equivalent JSON value.
pub fn hcl_to_json(hv: HclValue) -> Value {
    match hv {
        HclValue::Null => Value::Null,
        HclValue::Bool(b) => Value::Bool(b),
        HclValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(serde_json::Number::from(i))
            } else if let Some(u) = n.as_u64() {
                Value::Number(serde_json::Number::from(u))
            } else if let Some(f) = n.as_f64() {
                if let Some(sn) = serde_json::Number::from_f64(f) {
                    Value::Number(sn)
                } else {
                    Value::Null
                }
            } else {
                Value::Null
            }
        }
        HclValue::String(s) => Value::String(s),
        HclValue::Array(arr) => {
            let list: Vec<Value> = arr.into_iter().map(hcl_to_json).collect();
            Value::Array(list)
        }
        HclValue::Object(map) => {
            let mut json_map = serde_json::Map::new();
            for (k, v) in map {
                json_map.insert(k, hcl_to_json(v));
            }
            Value::Object(json_map)
        }
    }
}

/// Evaluates a string containing HCL interpolation templates (`${...}`).
pub fn evaluate_string(s: &str, ctx: &HclContext) -> Result<Option<Value>> {
    if !s.contains("${") && !s.contains("%{") {
        return Ok(None);
    }

    let template: hcl::Template = s
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse HCL template '{}': {:?}", s, e))?;
    let result = template
        .evaluate(ctx)
        .map_err(|e| anyhow::anyhow!("Failed to evaluate HCL template '{}': {:?}", s, e))?;

    tracing::info!("evaluate_string template result: {:?} for s: {}", result, s);

    let re = Regex::new(r"^\$\{([^}]+)\}$").unwrap();
    if let Some(caps) = re.captures(s) {
        let expr_str = caps.get(1).unwrap().as_str();
        let eval_res = evaluate_raw_expr(expr_str, ctx)?;
        tracing::info!(
            "evaluate_string raw_expr result: {:?} for expr_str: {}",
            eval_res,
            expr_str
        );
        return Ok(Some(eval_res));
    }

    Ok(Some(Value::String(result)))
}

/// Evaluates a raw HCL expression without the template wrapper.
pub fn evaluate_raw_expr(expr_str: &str, ctx: &HclContext) -> Result<Value> {
    let expr: hcl::Expression = expr_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse HCL expression '{}': {:?}", expr_str, e))?;
    let result = expr.evaluate(ctx).map_err(|e| {
        anyhow::anyhow!("Failed to evaluate HCL expression '{}': {:?}", expr_str, e)
    })?;
    Ok(hcl_to_json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hcl::eval::Context;
    use serde_json::json;

    #[test]
    fn test_json_to_hcl_and_back() {
        let original = json!({
            "string": "hello",
            "number": 123,
            "bool": true,
            "null": null,
            "array": [1, 2, 3],
            "object": {"nested": "value"}
        });

        let hcl_val = json_to_hcl(original.clone());
        let back = hcl_to_json(hcl_val);

        assert_eq!(original, back);
    }

    #[test]
    fn test_evaluate_string_simple() {
        let mut ctx = Context::new();
        ctx.declare_var("name", "world");

        let s = "hello ${name}";
        let result = evaluate_string(s, &ctx).unwrap().unwrap();
        assert_eq!(result, Value::String("hello world".to_string()));
    }

    #[test]
    fn test_evaluate_string_raw_expr() {
        let mut ctx = Context::new();
        ctx.declare_var("val", 42);

        let s = "${val}";
        let result = evaluate_string(s, &ctx).unwrap().unwrap();
        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_resolve_expressions_recursive() {
        let mut ctx = Context::new();
        ctx.declare_var("env", "prod");

        let mut val = json!({
            "service": "api-${env}",
            "tags": ["cloud", "${env}"],
            "config": {
                "db_suffix": "_${env}"
            }
        });

        resolve_expressions(&mut val, &ctx, true).unwrap();

        assert_eq!(val["service"], "api-prod");
        assert_eq!(val["tags"][1], "prod");
        assert_eq!(val["config"]["db_suffix"], "_prod");
    }
}
