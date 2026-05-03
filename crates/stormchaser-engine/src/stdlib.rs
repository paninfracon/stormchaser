use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use hcl::eval::{Context as HclContext, FuncArgs, FuncDef, ParamType};
use hcl::Value;

pub fn register_stdlib(ctx: &mut HclContext) {
    // String functions
    ctx.declare_func(
        "upper",
        FuncDef::builder().param(ParamType::String).build(upper),
    );
    ctx.declare_func(
        "lower",
        FuncDef::builder().param(ParamType::String).build(lower),
    );
    ctx.declare_func(
        "trim",
        FuncDef::builder()
            .param(ParamType::String)
            .param(ParamType::String)
            .build(trim),
    );
    ctx.declare_func(
        "trimspace",
        FuncDef::builder().param(ParamType::String).build(trimspace),
    );
    ctx.declare_func(
        "split",
        FuncDef::builder()
            .param(ParamType::String)
            .param(ParamType::String)
            .build(split),
    );
    ctx.declare_func(
        "join",
        FuncDef::builder()
            .param(ParamType::String)
            .param(ParamType::Array(Box::new(ParamType::Any)))
            .build(join),
    );
    ctx.declare_func(
        "replace",
        FuncDef::builder()
            .param(ParamType::String)
            .param(ParamType::String)
            .param(ParamType::String)
            .build(replace),
    );

    // Encoding functions
    ctx.declare_func(
        "jsonencode",
        FuncDef::builder().param(ParamType::Any).build(jsonencode),
    );
    ctx.declare_func(
        "jsondecode",
        FuncDef::builder()
            .param(ParamType::String)
            .build(jsondecode),
    );
    ctx.declare_func(
        "base64encode",
        FuncDef::builder()
            .param(ParamType::String)
            .build(base64encode),
    );
    ctx.declare_func(
        "base64decode",
        FuncDef::builder()
            .param(ParamType::String)
            .build(base64decode),
    );

    // Collection functions
    ctx.declare_func(
        "length",
        FuncDef::builder().param(ParamType::Any).build(length),
    );
    ctx.declare_func(
        "keys",
        FuncDef::builder()
            .param(ParamType::Object(Box::new(ParamType::Any)))
            .build(keys),
    );
    ctx.declare_func(
        "values",
        FuncDef::builder()
            .param(ParamType::Object(Box::new(ParamType::Any)))
            .build(values),
    );
    ctx.declare_func(
        "contains",
        FuncDef::builder()
            .param(ParamType::Array(Box::new(ParamType::Any)))
            .param(ParamType::Any)
            .build(contains),
    );

    // Logic/Type functions
    ctx.declare_func(
        "coalesce",
        FuncDef::builder()
            .variadic_param(ParamType::Any)
            .build(coalesce),
    );
}

fn upper(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::String(s)) = args.first() {
        Ok(Value::String(s.to_uppercase()))
    } else {
        Err("upper() expects a string argument".to_string())
    }
}

fn lower(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::String(s)) = args.first() {
        Ok(Value::String(s.to_lowercase()))
    } else {
        Err("lower() expects a string argument".to_string())
    }
}

fn trim(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("trim() expects exactly 2 arguments".to_string());
    }
    match (&args[0], &args[1]) {
        (Value::String(s), Value::String(cutset)) => {
            let cutset_chars: Vec<char> = cutset.chars().collect();
            Ok(Value::String(
                s.trim_matches(|c| cutset_chars.contains(&c)).to_string(),
            ))
        }
        _ => Err("trim() expects string arguments".to_string()),
    }
}

fn trimspace(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::String(s)) = args.first() {
        Ok(Value::String(s.trim().to_string()))
    } else {
        Err("trimspace() expects a string argument".to_string())
    }
}

fn split(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("split() expects exactly 2 arguments".to_string());
    }
    match (&args[0], &args[1]) {
        (Value::String(sep), Value::String(s)) => {
            let parts: Vec<Value> = s
                .split(sep)
                .map(|part| Value::String(part.to_string()))
                .collect();
            Ok(Value::Array(parts))
        }
        _ => Err("split() expects string arguments".to_string()),
    }
}

fn join(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("join() expects exactly 2 arguments".to_string());
    }
    match (&args[0], &args[1]) {
        (Value::String(sep), Value::Array(arr)) => {
            let mut strings = Vec::new();
            for item in arr {
                match item {
                    Value::String(s) => strings.push(s.clone()),
                    _ => strings.push(item.to_string()),
                }
            }
            Ok(Value::String(strings.join(sep)))
        }
        _ => Err("join() expects a string and an array".to_string()),
    }
}

fn replace(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 3 {
        return Err("replace() expects exactly 3 arguments".to_string());
    }
    match (&args[0], &args[1], &args[2]) {
        (Value::String(s), Value::String(old), Value::String(new)) => {
            Ok(Value::String(s.replace(old, new)))
        }
        _ => Err("replace() expects string arguments".to_string()),
    }
}

fn jsonencode(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("jsonencode() expects exactly 1 argument".to_string());
    }
    match serde_json::to_string(&args[0]) {
        Ok(s) => Ok(Value::String(s)),
        Err(e) => Err(format!("jsonencode() failed: {}", e)),
    }
}

fn jsondecode(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::String(s)) = args.first() {
        match serde_json::from_str::<Value>(s) {
            Ok(v) => Ok(v),
            Err(e) => Err(format!("jsondecode() failed: {}", e)),
        }
    } else {
        Err("jsondecode() expects a string argument".to_string())
    }
}

fn base64encode(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::String(s)) = args.first() {
        Ok(Value::String(general_purpose::STANDARD.encode(s)))
    } else {
        Err("base64encode() expects a string argument".to_string())
    }
}

fn base64decode(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::String(s)) = args.first() {
        match general_purpose::STANDARD.decode(s) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(decoded) => Ok(Value::String(decoded)),
                Err(_) => Err("base64decode() resulted in invalid UTF-8".to_string()),
            },
            Err(e) => Err(format!("base64decode() failed: {}", e)),
        }
    } else {
        Err("base64decode() expects a string argument".to_string())
    }
}

fn length(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("length() expects exactly 1 argument".to_string());
    }
    match &args[0] {
        Value::String(s) => Ok(Value::Number(hcl::Number::from(s.len() as u64))),
        Value::Array(a) => Ok(Value::Number(hcl::Number::from(a.len() as u64))),
        Value::Object(o) => Ok(Value::Number(hcl::Number::from(o.len() as u64))),
        _ => Err("length() expects a string, array, or object".to_string()),
    }
}

fn keys(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::Object(o)) = args.first() {
        let keys_arr: Vec<Value> = o.keys().map(|k| Value::String(k.to_string())).collect();
        Ok(Value::Array(keys_arr))
    } else {
        Err("keys() expects an object argument".to_string())
    }
}

fn values(args: FuncArgs) -> Result<Value, String> {
    if let Some(Value::Object(o)) = args.first() {
        let vals_arr: Vec<Value> = o.values().cloned().collect();
        Ok(Value::Array(vals_arr))
    } else {
        Err("values() expects an object argument".to_string())
    }
}

fn contains(args: FuncArgs) -> Result<Value, String> {
    if args.len() != 2 {
        return Err("contains() expects exactly 2 arguments".to_string());
    }
    if let Value::Array(arr) = &args[0] {
        let target = &args[1];
        Ok(Value::Bool(arr.contains(target)))
    } else {
        Err("contains() expects an array as its first argument".to_string())
    }
}

fn coalesce(args: FuncArgs) -> Result<Value, String> {
    if args.is_empty() {
        return Err("coalesce() expects at least 1 argument".to_string());
    }
    for arg in args.iter() {
        if !arg.is_null() {
            // Also treat empty string as null in coalesce if we follow some terraform semantics,
            // but standard HCL coalesce returns the first non-null value.
            if let Value::String(s) = arg {
                if s.is_empty() {
                    continue;
                }
            }
            return Ok(arg.clone());
        }
    }
    Err("coalesce() no non-null/non-empty arguments found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hcl_eval::evaluate_raw_expr;
    use hcl::eval::Context;
    use serde_json::json;

    fn eval_with_stdlib(expr: &str) -> serde_json::Value {
        let mut ctx = Context::new();
        register_stdlib(&mut ctx);
        evaluate_raw_expr(expr, &ctx).expect("evaluation failed")
    }

    #[test]
    fn test_string_functions() {
        assert_eq!(eval_with_stdlib("upper(\"hello\")"), json!("HELLO"));
        assert_eq!(eval_with_stdlib("lower(\"HELLO\")"), json!("hello"));
        assert_eq!(
            eval_with_stdlib("trim(\"?!hello?!\", \"?!\")"),
            json!("hello")
        );
        assert_eq!(
            eval_with_stdlib("trimspace(\"  hello  \\n\")"),
            json!("hello")
        );

        let split_res = eval_with_stdlib("split(\",\", \"a,b,c\")");
        assert_eq!(split_res, json!(["a", "b", "c"]));

        assert_eq!(
            eval_with_stdlib("join(\"-\", [\"a\", \"b\", \"c\"])"),
            json!("a-b-c")
        );
        assert_eq!(
            eval_with_stdlib("replace(\"hello world\", \"world\", \"there\")"),
            json!("hello there")
        );
    }

    #[test]
    fn test_encoding_functions() {
        let encode_res = eval_with_stdlib("jsonencode({\"a\" = 1})");
        assert_eq!(encode_res, json!("{\"a\":1}"));

        let decode_res = eval_with_stdlib("jsondecode(\"{\\\"a\\\": 1}\")");
        assert_eq!(decode_res, json!({"a": 1}));

        assert_eq!(
            eval_with_stdlib("base64encode(\"hello\")"),
            json!("aGVsbG8=")
        );
        assert_eq!(
            eval_with_stdlib("base64decode(\"aGVsbG8=\")"),
            json!("hello")
        );
    }

    #[test]
    fn test_collection_functions() {
        assert_eq!(eval_with_stdlib("length(\"hello\")"), json!(5));
        assert_eq!(eval_with_stdlib("length([1, 2, 3])"), json!(3));

        assert_eq!(
            eval_with_stdlib("contains([\"a\", \"b\"], \"b\")"),
            json!(true)
        );
        assert_eq!(
            eval_with_stdlib("contains([\"a\", \"b\"], \"c\")"),
            json!(false)
        );
    }

    #[test]
    fn test_logic_functions() {
        assert_eq!(
            eval_with_stdlib("coalesce(\"\", null, \"first\", \"second\")"),
            json!("first")
        );
    }
}
