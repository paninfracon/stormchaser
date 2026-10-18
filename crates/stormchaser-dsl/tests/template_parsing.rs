use stormchaser_dsl::StormchaserParser;

#[test]
fn test_parse_step_library() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_libs" {
            step_library "ubuntu_base" {
                type = "RunContainer"
                params {
                    tag = "latest"
                }
                spec {
                    image = "ubuntu:22.04"
                }
            }

            steps {
                step "my_step" "ubuntu_base" {
                    params {
                        tag = "v1"
                    }
                    command = ["echo", "hello"]
                }
            }
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");

    assert_eq!(workflow.step_libraries.len(), 1);
    let lib = &workflow.step_libraries[0];
    assert_eq!(lib.name, "ubuntu_base");
    assert_eq!(lib.r#type, "RunContainer");
    assert_eq!(lib.params.get("tag").unwrap(), "latest");
    assert_eq!(lib.spec["image"], "ubuntu:22.04");

    // The step is the library's step, with its own settings on top
    let step = &workflow.steps[0];
    assert_eq!(step.r#type, "RunContainer");
    assert_eq!(step.params.get("tag").unwrap(), "v1");
    assert_eq!(step.spec["command"][0], "echo");
    assert_eq!(step.spec["image"], "ubuntu:22.04");
}

#[test]
fn test_parse_workflow_template_and_include() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow_template "my_template" {
            input "repo" {
                type = "string"
            }
            steps {
                step "build" "RunContainer" {
                    spec {
                        image = "golang:1.20"
                    }
                }
            }
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse template DSL");
    assert!(workflow.is_template);
    assert_eq!(workflow.inputs.len(), 1);
    assert_eq!(workflow.inputs[0].name, "repo");

    let main_dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "main_workflow" {
            include "shared_build" {
                workflow = "my_template.storm"
                inputs = {
                    repo = "my_repo"
                }
            }
        }
    "#;

    let main_workflow = parser.parse(main_dsl).expect("Failed to parse main DSL");
    assert!(!main_workflow.is_template);
    assert_eq!(main_workflow.includes.len(), 1);
    let inc = &main_workflow.includes[0];
    assert_eq!(inc.name, "shared_build");
    assert_eq!(inc.workflow, "my_template.storm");
    assert_eq!(inc.inputs.get("repo").unwrap(), "my_repo");
}

#[test]
fn test_parse_terraform_pipeline() {
    let dsl = std::fs::read_to_string("../../tests/terraform-pipeline.storm").unwrap();
    let parser = stormchaser_dsl::StormchaserParser::new();
    let workflow = parser
        .parse(&dsl)
        .expect("Failed to parse terraform-pipeline.storm");
    assert!(!workflow.steps.is_empty());
}

#[test]
fn test_spec_attribute_means_the_same_as_spec_block() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "spec_forms" {
            step_library "notify_base" {
                type = "WebhookInvoke"
                spec = {
                    method = "PUT"
                }
            }

            steps {
                step "as_attribute" "WebhookInvoke" {
                    spec = {
                        url = "https://paninfracon.net/hook"
                    }
                }
                step "as_block" "WebhookInvoke" {
                    spec {
                        url = "https://paninfracon.net/hook"
                    }
                }
            }
        }
    "#;

    let workflow = StormchaserParser::new()
        .parse(dsl)
        .expect("Failed to parse DSL");

    assert_eq!(workflow.step_libraries[0].spec["method"], "PUT");
    assert_eq!(workflow.steps[0].spec, workflow.steps[1].spec);
    assert_eq!(
        workflow.steps[0].spec,
        serde_json::json!({ "url": "https://paninfracon.net/hook" })
    );
}

#[test]
fn a_spec_attribute_that_is_not_an_object_is_kept_in_steps_and_step_libraries_alike() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "spec_value" {
            step_library "from_input" {
                type = "WebhookInvoke"
                spec = "${inputs.webhook}"
            }

            steps {
                step "from_input" "WebhookInvoke" {
                    spec = "${inputs.webhook}"
                }
            }
        }
    "#;

    let workflow = StormchaserParser::new()
        .parse(dsl)
        .expect("Failed to parse DSL");

    assert_eq!(workflow.steps[0].spec["spec"], "${inputs.webhook}");
    assert_eq!(workflow.step_libraries[0].spec, workflow.steps[0].spec);
}
