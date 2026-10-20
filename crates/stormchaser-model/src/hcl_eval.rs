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
#[expect(
    clippy::disallowed_methods,
    reason = "parsed and evaluated inside hcl_guard::guarded"
)]
pub fn evaluate_string(s: &str, ctx: &HclContext) -> Result<Option<Value>> {
    if !s.contains("${") && !s.contains("%{") {
        return Ok(None);
    }

    // A template comes from a workflow author and can nest without limit; see
    // `hcl_guard` for why it is parsed and evaluated under a guard
    let result = crate::hcl_guard::guarded(s, crate::hcl_guard::expr_limit(), || {
        let template: hcl::Template = s
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse HCL template '{}': {:?}", s, e))?;
        template
            .evaluate(ctx)
            .map_err(|e| anyhow::anyhow!("Failed to evaluate HCL template '{}': {:?}", s, e))
    })?;

    // Results are not logged: they carry resolved secrets and inputs.
    let re = Regex::new(r"^\$\{([^}]+)\}$").unwrap();
    if let Some(caps) = re.captures(s) {
        let expr_str = caps.get(1).unwrap().as_str();
        return Ok(Some(evaluate_raw_expr(expr_str, ctx)?));
    }

    Ok(Some(Value::String(result)))
}

/// Evaluates a raw HCL expression without the template wrapper.
#[expect(
    clippy::disallowed_methods,
    reason = "parsed and evaluated inside hcl_guard::guarded"
)]
pub fn evaluate_raw_expr(expr_str: &str, ctx: &HclContext) -> Result<Value> {
    crate::hcl_guard::guarded(expr_str, crate::hcl_guard::expr_limit(), || {
        let expr: hcl::Expression = expr_str.parse().map_err(|e| {
            anyhow::anyhow!("Failed to parse HCL expression '{}': {:?}", expr_str, e)
        })?;
        let result = expr.evaluate(ctx).map_err(|e| {
            anyhow::anyhow!("Failed to evaluate HCL expression '{}': {:?}", expr_str, e)
        })?;
        // Checked here, where there is room to drop a result that is too deep
        crate::hcl_guard::ensure_shallow(hcl_to_json(result))
    })
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

    /// Text that used to end the process with a stack overflow. Each case is run on a
    /// thread with the 2 MiB stack a server's worker threads have.
    #[test]
    fn hostile_nesting_is_evaluated_or_refused_but_never_overflows() {
        let on_small_stack = |f: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .stack_size(2 * 1024 * 1024)
                .spawn(f)
                .unwrap()
                .join()
                .expect("the evaluation crashed");
        };

        on_small_stack(Box::new(|| {
            let ctx = Context::new();
            // Within the limit: evaluated, on a stack that is big enough
            let nots = format!("{}true", "!".repeat(3000));
            assert_eq!(evaluate_raw_expr(&nots, &ctx).unwrap(), json!(true));
            let parens = format!("{}1{}", "(".repeat(3000), ")".repeat(3000));
            assert_eq!(evaluate_raw_expr(&parens, &ctx).unwrap(), json!(1));
            let sum = format!("1{}", " + 1".repeat(3000));
            assert_eq!(evaluate_raw_expr(&sum, &ctx).unwrap(), json!(3001));

            // Within the limit but producing a value too deep to hand on
            let lists = format!("{}1{}", "[".repeat(3000), "]".repeat(3000));
            let err = evaluate_raw_expr(&lists, &ctx).unwrap_err();
            assert!(err.to_string().contains("nested too deeply"), "{err}");

            // Over the limit: refused unread
            for hostile in [
                "!".repeat(50_000) + "true",
                "(".repeat(50_000),
                "-".repeat(1_000_000),
            ] {
                let err = evaluate_raw_expr(&hostile, &ctx).unwrap_err();
                assert!(err.to_string().contains("too complex"));
            }
        }));

        on_small_stack(Box::new(|| {
            let ctx = Context::new();
            let nested = format!("{}1{}", "\"${".repeat(2000), "}\"".repeat(2000));
            assert!(evaluate_string(&nested, &ctx).is_ok());
            let hostile = format!("${{{}true}}", "!".repeat(50_000));
            assert!(evaluate_string(&hostile, &ctx).is_err());
            // An ordinary template is unaffected
            assert_eq!(
                evaluate_string("build-${1 + 1}", &ctx).unwrap(),
                Some(json!("build-2"))
            );
        }));
    }
}
