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

    let step = &workflow.steps[0];
    assert_eq!(step.r#type, "ubuntu_base");
    assert_eq!(step.params.get("tag").unwrap(), "v1");
    assert_eq!(step.spec["command"][0], "echo");
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
