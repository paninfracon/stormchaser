//! A step that names a step library as its type is the library's step, with what the
//! step sets itself taking precedence, and the library's params are its arguments.

use serde_json::json;
use stormchaser_dsl::StormchaserParser;

const LIBRARY: &str = r#"
    step_library "python_env" {
        type = "RunContainer"
        params {
            python_version = "3.11"
            retries        = 3
        }
        spec {
            image  = "python:${params.python_version}-slim"
            cpu    = "500m"
            env    = [{ name = "RETRIES", value = "${params.retries}" }]
        }
        timeout       = "10m"
        allow_failure = true
        retry {
            count = 2
        }
    }
"#;

fn parse(steps: &str) -> anyhow::Result<stormchaser_dsl::ast::Workflow> {
    StormchaserParser::new().parse(&format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "libraries" {{
            {LIBRARY}
            steps {{
                {steps}
            }}
        }}
        "#
    ))
}

#[test]
fn a_step_is_its_librarys_step_with_its_own_settings_on_top() {
    let workflow = parse(
        r#"
        step "tests" "python_env" {
            params {
                python_version = "3.12"
            }
            spec {
                command = ["pytest"]
                cpu     = "1"
            }
            timeout = "5m"
            retry {
                count   = 4
                backoff = "linear"
            }
        }
        "#,
    )
    .unwrap();

    let step = &workflow.steps[0];
    assert_eq!(step.r#type, "RunContainer");
    assert_eq!(step.params["python_version"], "3.12");
    // A number is kept as its text
    assert_eq!(step.params["retries"], "3");
    assert_eq!(
        step.spec,
        json!({
            "image": "python:${params.python_version}-slim",
            "cpu": "1",
            "env": [{ "name": "RETRIES", "value": "${params.retries}" }],
            "command": ["pytest"],
        })
    );
    assert_eq!(step.timeout.as_deref(), Some("5m"));
    assert_eq!(step.allow_failure, Some(true));
    let retry = step.retry.as_ref().unwrap();
    assert_eq!((retry.count, retry.backoff.as_str()), (4, "linear"));
}

#[test]
fn a_step_that_sets_nothing_takes_the_librarys_defaults() {
    let workflow = parse(r#"step "lint" "python_env" {}"#).unwrap();
    let step = &workflow.steps[0];
    assert_eq!(step.params["python_version"], "3.11");
    assert_eq!(step.timeout.as_deref(), Some("10m"));
    assert_eq!(step.retry.as_ref().map(|r| r.count), Some(2));
}

#[test]
fn a_param_that_neither_step_nor_library_sets_is_an_error() {
    let err = parse(
        r#"
        step "tests" "python_env" {
            spec {
                command = ["pytest", "-k", "${params.selection}"]
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains(
            "Step 'tests' uses params.selection, which neither it nor its step library 'python_env' sets"
        ),
        "{err:#}"
    );
}

#[test]
fn a_param_must_be_a_single_value() {
    let err = parse(
        r#"
        step "tests" "python_env" {
            params {
                versions = ["3.11", "3.12"]
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("Param 'versions' must be a string, number or boolean"),
        "{err:#}"
    );
}

#[test]
fn steps_inside_groups_and_failure_handlers_take_their_library_too() {
    let workflow = StormchaserParser::new()
        .parse(&format!(
            r#"
            stormchaser_dsl_version = "v1"
            workflow "libraries" {{
                {LIBRARY}
                steps {{
                    step "group" "Parallel" {{
                        steps {{
                            step "inner" "python_env" {{}}
                        }}
                    }}
                }}
                on_failure {{
                    step "report" "python_env" {{}}
                }}
            }}
            "#
        ))
        .unwrap();
    let inner = &workflow.steps[0].steps.as_ref().unwrap()[0];
    assert_eq!(inner.r#type, "RunContainer");
    assert_eq!(
        workflow.on_failure.as_ref().unwrap()[0].r#type,
        "RunContainer"
    );
}
