use crate::secrets::SharedSecretBackend;
use anyhow::Result;
use hcl::eval::{Context as HclContext, FuncDef, ParamType};
use hcl::Value as HclValue;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde_json::Value;
use stormchaser_model::RunId;

pub use stormchaser_model::hcl_eval::{
    evaluate_raw_expr, evaluate_string, hcl_to_json, json_to_hcl, resolve_expressions,
};

static SECRETS_BACKEND: Lazy<RwLock<Option<SharedSecretBackend>>> = Lazy::new(|| RwLock::new(None));

/// Sets the global secrets backend used for resolving `secrets.*` references in HCL expressions.
pub fn set_secrets_backend(backend: SharedSecretBackend) {
    let mut lock = SECRETS_BACKEND.write();
    *lock = Some(backend);
}

thread_local! {
    /// Values `secret()` returned while [`capturing_secrets`] runs on this thread.
    static RESOLVED_SECRETS: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Secrets fetched while workflow aliases were evaluated, by run, until the run's next
/// step is scheduled and records them.
static ALIAS_SECRETS: Lazy<parking_lot::Mutex<std::collections::HashMap<RunId, Vec<String>>>> =
    Lazy::new(Default::default);

fn record_resolved_secret(value: &str) {
    RESOLVED_SECRETS.with(|cell| {
        if let Some(values) = cell.borrow_mut().as_mut() {
            values.push(value.to_string());
        }
    });
}

/// Runs `work` and returns, with its result, every value `secret()` fetched meanwhile.
///
/// A run's secrets are masked in what the API returns only if the run records them.
/// Values in its SOPS file are recorded when it starts; a value fetched from Vault with
/// `secret()` is only known once an expression asks for it, so callers that evaluate a
/// run's expressions record what this returns (`db::runs::add_sensitive_values`).
/// Evaluation can switch stacks (`hcl_guard`) but stays on this thread.
pub fn capturing_secrets<T>(work: impl FnOnce() -> T) -> (T, Vec<String>) {
    let outer = RESOLVED_SECRETS.with(|cell| cell.borrow_mut().replace(Vec::new()));
    let result = work();
    let mut captured = RESOLVED_SECRETS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let captured = slot.take().unwrap_or_default();
        *slot = outer;
        captured
    });
    // A capture nested in another is seen by the outer one too
    RESOLVED_SECRETS.with(|cell| {
        if let Some(values) = cell.borrow_mut().as_mut() {
            values.extend(captured.iter().cloned());
        }
    });
    captured.dedup();
    (result, captured)
}

/// Takes the secrets fetched for `run_id` while its aliases were evaluated.
pub fn take_alias_secrets(run_id: RunId) -> Vec<String> {
    ALIAS_SECRETS.lock().remove(&run_id).unwrap_or_default()
}

fn secret_lookup(args: hcl::eval::FuncArgs) -> Result<HclValue, String> {
    if args.len() != 1 {
        return Err("secret() expects exactly 1 argument".to_string());
    }

    let path_and_key_str = match &args[0] {
        HclValue::String(s) => s.to_string(),
        _ => return Err("secret() expects a string argument".to_string()),
    };

    let backend = {
        let lock = SECRETS_BACKEND.read();
        lock.clone()
    };

    if let Some(backend) = backend {
        let handle = tokio::runtime::Handle::current();

        let (path, key) = match path_and_key_str.split_once('#') {
            Some((p, k)) => (p.to_string(), k.to_string()),
            None => {
                return Err(format!(
                    "Invalid secret lookup format (expected path#key): {}",
                    path_and_key_str
                ));
            }
        };

        if let Err(e) = crate::secrets::ensure_secret_path_allowed(&path) {
            return Err(e.to_string());
        }

        let val = match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(move || {
                handle.block_on(async move { backend.get_secret(&path, &key).await })
            }),
            _ => handle.block_on(async move { backend.get_secret(&path, &key).await }),
        };

        match val {
            Ok(val) => {
                record_resolved_secret(&val);
                Ok(HclValue::String(val))
            }
            Err(e) => Err(format!(
                "Secret lookup failed for {}: {:?}",
                path_and_key_str, e
            )),
        }
    } else {
        Err("No secrets backend configured".to_string())
    }
}

/// Create context.
pub fn create_context(
    inputs: Value,
    run_id: RunId,
    secrets: Value,
    steps: Value,
    workflow: Option<&stormchaser_dsl::ast::Workflow>,
    step: Option<&stormchaser_dsl::ast::Step>,
) -> HclContext<'static> {
    let mut ctx = HclContext::new();
    ctx.declare_var("inputs", json_to_hcl(inputs));
    ctx.declare_var("secrets", json_to_hcl(secrets));
    ctx.declare_var(
        "run",
        json_to_hcl(serde_json::json!({"id": run_id.to_string()})),
    );
    ctx.declare_var("steps", json_to_hcl(steps));

    ctx.declare_func("secret", FuncDef::new(secret_lookup, [ParamType::Any]));
    crate::stdlib::register_stdlib(&mut ctx);

    let ((), alias_secrets) = capturing_secrets(|| apply_aliases(&mut ctx, workflow, step));
    if !alias_secrets.is_empty() {
        ALIAS_SECRETS
            .lock()
            .entry(run_id)
            .or_default()
            .extend(alias_secrets);
    }

    ctx
}

fn apply_aliases(
    ctx: &mut HclContext<'static>,
    workflow: Option<&stormchaser_dsl::ast::Workflow>,
    step: Option<&stormchaser_dsl::ast::Step>,
) {
    // Apply workflow aliases
    if let Some(wf) = workflow {
        for (alias, expr_str) in &wf.aliases {
            let val = match evaluate_string(expr_str, ctx) {
                Ok(Some(v)) => v,
                Ok(None) => Value::String(expr_str.clone()),
                Err(e) => {
                    tracing::warn!("Failed to evaluate workflow alias '{}': {:?}", alias, e);
                    Value::String(expr_str.clone())
                }
            };
            ctx.declare_var(alias.clone(), json_to_hcl(val));
        }
    }

    // Apply step aliases
    if let Some(st) = step {
        for (alias, expr_str) in &st.aliases {
            let val = match evaluate_string(expr_str, ctx) {
                Ok(Some(v)) => v,
                Ok(None) => Value::String(expr_str.clone()),
                Err(e) => {
                    tracing::warn!("Failed to evaluate step alias '{}': {:?}", alias, e);
                    Value::String(expr_str.clone())
                }
            };
            ctx.declare_var(alias.clone(), json_to_hcl(val));
        }
    }
}

/// The fields of a step's spec that the engine renders as Jinja templates when the step
/// runs, as JSON pointers into the spec.
fn jinja_template_fields(step_type: &str) -> &'static [&'static str] {
    match step_type {
        "SqlExecute" => &["/query"],
        "JinjaRender" | "TestReportEmail" => &["/template"],
        "RestApi" | "WebhookInvoke" | "EmailSend" => &["/body"],
        "Approval" | "TerraformApproval" => &["/notify/body"],
        _ => &[],
    }
}

/// Resolves a step's params, then its spec, in which they are `params.<name>`. An
/// expression that cannot be resolved is left as it is.
pub fn resolve_step(step_type: &str, spec: &mut Value, params: &mut Value, ctx: &HclContext) {
    let _ = resolve_expressions(params, ctx, false);
    let mut step_ctx = ctx.clone();
    step_ctx.declare_var("params", json_to_hcl(params.clone()));
    let _ = resolve_step_spec(step_type, spec, &step_ctx);
}

/// Resolves the `${...}` expressions in a step's spec.
///
/// A field the step later renders as a Jinja template gets each interpolated value
/// written as a Jinja string literal, so that rendering prints it as the text it is.
/// Spliced in as is, an input holding `{{ ... }}` or `{% ... %}` would be run as
/// template code in that second pass. Those fields are left out of the plain pass,
/// which would otherwise evaluate a `${...}` an interpolated value contains.
pub fn resolve_step_spec(step_type: &str, spec: &mut Value, ctx: &HclContext) -> Result<()> {
    let mut templates = Vec::new();
    for field in jinja_template_fields(step_type) {
        for pointer in [field.to_string(), format!("/spec{field}")] {
            if let Some(slot) = spec.pointer_mut(&pointer) {
                if let Value::String(text) = slot.take() {
                    templates.push((pointer, text));
                }
            }
        }
    }

    // Not strict: a `${...}` that is not an expression, such as a shell variable, is
    // left as it is, and the expressions after it are still resolved
    let result = resolve_expressions(spec, ctx, false);

    for (pointer, text) in templates {
        let resolved = resolve_jinja_template(&text, ctx).unwrap_or(text);
        if let Some(slot) = spec.pointer_mut(&pointer) {
            *slot = Value::String(resolved);
        }
    }
    result
}

const JINJA_LITERAL_FUNC: &str = "stormchaser_jinja_literal";

/// Evaluates the `${...}` and `%{...}` sequences of a Jinja template, writing each
/// interpolated value as a Jinja string literal. A template with neither is returned
/// unchanged.
pub fn resolve_jinja_template(text: &str, ctx: &HclContext) -> Result<String> {
    use hcl::eval::Evaluate;
    if !text.contains("${") && !text.contains("%{") {
        return Ok(text.to_string());
    }
    stormchaser_model::hcl_guard::guarded(text, stormchaser_model::hcl_guard::expr_limit(), || {
        let mut template: hcl::Template = text
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse HCL template: {:?}", e))?;
        wrap_interpolations(&mut template);
        let mut ctx = ctx.clone();
        ctx.declare_func(
            JINJA_LITERAL_FUNC,
            FuncDef::new(
                |args: hcl::eval::FuncArgs| {
                    let text = match &args[0] {
                        HclValue::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    Ok(HclValue::String(jinja_string_literal(&text)))
                },
                [ParamType::Any],
            ),
        );
        template
            .evaluate(&ctx)
            .map_err(|e| anyhow::anyhow!("Failed to evaluate HCL template: {:?}", e))
    })
}

/// Routes every interpolation of a template, including those inside `%{if}` and
/// `%{for}`, through [`JINJA_LITERAL_FUNC`].
fn wrap_interpolations(template: &mut hcl::Template) {
    use hcl::template::{Directive, Element};
    for element in template.elements_mut() {
        match element {
            Element::Literal(_) => {}
            Element::Interpolation(interpolation) => {
                let expr = std::mem::replace(&mut interpolation.expr, hcl::Expression::Null);
                interpolation.expr = hcl::expr::FuncCall::builder(JINJA_LITERAL_FUNC)
                    .arg(expr)
                    .build()
                    .into();
            }
            Element::Directive(directive) => match &mut **directive {
                Directive::If(directive) => {
                    wrap_interpolations(&mut directive.true_template);
                    if let Some(false_template) = &mut directive.false_template {
                        wrap_interpolations(false_template);
                    }
                }
                Directive::For(directive) => wrap_interpolations(&mut directive.template),
            },
        }
    }
}

/// `text` as a Jinja expression that renders as exactly `text`.
fn jinja_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str("{{ \"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str("\" }}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MockBackend;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn test_secret_function() {
        let mut secrets = HashMap::new();
        secrets.insert("kv/db:password".to_string(), "secret-password".to_string());
        secrets.insert(
            "infra/root:token".to_string(),
            "not-for-workflows".to_string(),
        );
        std::env::set_var(crate::secrets::VAULT_ALLOWED_PATHS_ENV, "kv/*");
        let backend = Arc::new(MockBackend::new(secrets)) as SharedSecretBackend;
        set_secrets_backend(backend);

        let ctx = create_context(
            serde_json::json!({}),
            RunId::new_v4(),
            serde_json::json!({}),
            serde_json::json!({}),
            None,
            None,
        );

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let result = evaluate_raw_expr("secret(\"kv/db#password\")", &ctx).unwrap();
            assert_eq!(result, serde_json::json!("secret-password"));

            // The backend holds it, but the operator has not listed the path
            assert!(evaluate_raw_expr("secret(\"infra/root#token\")", &ctx).is_err());

            // What secret() fetches is reported to whoever is capturing, nested or not,
            // and to no one otherwise
            let (value, fetched) = capturing_secrets(|| {
                let (inner, nested) = capturing_secrets(|| {
                    evaluate_string("db=${secret(\"kv/db#password\")}", &ctx).unwrap()
                });
                assert_eq!(nested, vec!["secret-password".to_string()]);
                inner
            });
            assert_eq!(value, Some(serde_json::json!("db=secret-password")));
            assert_eq!(fetched, vec!["secret-password".to_string()]);
            let (_, outside) = capturing_secrets(|| ());
            assert!(outside.is_empty());

            // Through a workflow alias: kept for the run until its next step records it
            let run_id = RunId::new_v4();
            let workflow: stormchaser_dsl::ast::Workflow =
                serde_json::from_value(serde_json::json!({
                    "dsl_version": "v1",
                    "name": "w",
                "steps": [],
                "handlers": [],
                "outputs": [],
                "inputs": [],
                "storage": [],
                "libraries": [],
                    "aliases": { "db": "${secret(\"kv/db#password\")}" },
                }))
                .expect("a minimal workflow");
            let _ = create_context(
                serde_json::json!({}),
                run_id,
                serde_json::json!({}),
                serde_json::json!({}),
                Some(&workflow),
                None,
            );
            assert_eq!(
                take_alias_secrets(run_id),
                vec!["secret-password".to_string()]
            );
            assert!(take_alias_secrets(run_id).is_empty());
        });
    }

    fn render(template: &str, ctx: &Value) -> String {
        minijinja::Environment::new()
            .render_str(template, minijinja::value::Serde(ctx))
            .unwrap()
    }

    fn context_with(inputs: Value) -> HclContext<'static> {
        create_context(
            inputs,
            RunId::new_v4(),
            serde_json::json!({}),
            serde_json::json!({}),
            None,
            None,
        )
    }

    #[test]
    fn any_text_survives_a_jinja_string_literal() {
        for text in [
            "",
            "plain",
            "{{ inputs.secret }}",
            "{% for x in range(99999999) %}x{% endfor %}",
            "{# comment #}",
            "\" }}{{ inputs.secret }}{{ \"",
            "back\\slash \\\" \\n",
            "{% raw %}{% endraw %}",
            "line\nbreak\r\ttab\u{7}bell",
            "unicode ✓ 🚨 \u{2028}",
        ] {
            assert_eq!(
                render(&jinja_string_literal(text), &serde_json::json!({})),
                text
            );
        }
    }

    #[test]
    fn an_input_is_spliced_into_a_template_as_text_not_as_template_code() {
        let ctx = context_with(serde_json::json!({
            "name": "{{ inputs.token }}{% for x in [1,2] %}!{% endfor %}",
            "items": ["a", "{{ run.id }}"],
            "token": "should-not-appear",
        }));
        let jinja_ctx = serde_json::json!({ "inputs": { "token": "should-not-appear" }, "run": { "id": "r1" } });

        let resolved =
            resolve_jinja_template("Hello ${inputs.name} from {{ run.id }}", &ctx).unwrap();
        assert_eq!(
            render(&resolved, &jinja_ctx),
            "Hello {{ inputs.token }}{% for x in [1,2] %}!{% endfor %} from r1"
        );

        // Inside a directive as well
        let resolved =
            resolve_jinja_template("%{ for i in inputs.items }[${i}]%{ endfor }", &ctx).unwrap();
        assert_eq!(render(&resolved, &jinja_ctx), "[a][{{ run.id }}]");

        // Numbers and booleans as HCL would write them; text with no expressions as is
        let resolved = resolve_jinja_template("${1 + 1} ${true}", &ctx).unwrap();
        assert_eq!(render(&resolved, &jinja_ctx), "2 true");
        assert_eq!(
            resolve_jinja_template("{{ run.id }}", &ctx).unwrap(),
            "{{ run.id }}"
        );
    }

    #[test]
    fn only_template_fields_are_resolved_for_jinja_and_no_value_is_evaluated_twice() {
        let ctx = context_with(serde_json::json!({
            "query": "x' OR {{ 1 }}",
            "nested": "${inputs.other}",
            "other": "leaked",
        }));
        let mut spec = serde_json::json!({
            "connection": "db-${inputs.other}",
            "query": "SELECT '${inputs.query}', '${inputs.nested}'",
        });
        resolve_step_spec("SqlExecute", &mut spec, &ctx).unwrap();
        assert_eq!(spec["connection"], "db-leaked");
        assert_eq!(
            render(spec["query"].as_str().unwrap(), &serde_json::json!({})),
            "SELECT 'x' OR {{ 1 }}', '${inputs.other}'"
        );

        // The same field under a nested `spec`, and a step type with no templates
        let mut spec = serde_json::json!({ "spec": { "body": "${inputs.query}" } });
        resolve_step_spec("WebhookInvoke", &mut spec, &ctx).unwrap();
        assert_eq!(spec["spec"]["body"], "{{ \"x' OR {{ 1 }}\" }}");
        let mut spec = serde_json::json!({ "body": "${inputs.query}" });
        resolve_step_spec("RunContainer", &mut spec, &ctx).unwrap();
        assert_eq!(spec["body"], "x' OR {{ 1 }}");

        let mut spec = serde_json::json!({ "notify": { "body": "${inputs.query}", "subject": "${inputs.other}" } });
        resolve_step_spec("Approval", &mut spec, &ctx).unwrap();
        assert_eq!(spec["notify"]["subject"], "leaked");
        assert_eq!(
            render(
                spec["notify"]["body"].as_str().unwrap(),
                &serde_json::json!({})
            ),
            "x' OR {{ 1 }}"
        );
    }
}
