use stormchaser_dsl::StormchaserParser;

#[test]
fn test_parse_markers() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_markers" {
            steps {
                step "producer" "RunContainer" {
                    image = "alpine"
                    outputs {
                        start_marker = "BEGIN"
                        end_marker = "END"
                        output "res" {
                            source = "logs"
                            regex = "RES: (.*)"
                        }
                    }
                }
            }
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");
    let step = &workflow.steps[0];

    assert_eq!(step.start_marker.as_deref(), Some("BEGIN"));
    assert_eq!(step.end_marker.as_deref(), Some("END"));
    assert_eq!(step.outputs.len(), 1);
    assert_eq!(step.outputs[0].name, "res");
}

#[test]
fn test_parse_run_k8s_job() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_k8s" {
            steps {
                step "k8s_job" "RunK8sJob" {
                    image = "custom-image"
                    completions = 3
                    parallelism = 2
                    node_selector = {
                        disktype = "ssd"
                    }
                }
            }
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");
    let step = &workflow.steps[0];

    assert_eq!(step.r#type, "RunK8sJob");
    let spec = &step.spec;
    assert_eq!(spec["image"], "custom-image");
    assert_eq!(spec["completions"], 3);
    assert_eq!(spec["parallelism"], 2);
    assert_eq!(spec["node_selector"]["disktype"], "ssd");
}

#[test]
fn test_parse_run_container() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_common" {
            steps {
                step "common_job" "RunContainer" {
                    image = "alpine"
                    command = ["echo"]
                    args = ["hello"]
                    cpu = "100m"
                    memory = "128Mi"
                }
            }
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");
    let step = &workflow.steps[0];

    assert_eq!(step.r#type, "RunContainer");
    let spec = &step.spec;
    assert_eq!(spec["image"], "alpine");
    assert_eq!(spec["command"][0], "echo");
    assert_eq!(spec["args"][0], "hello");
    assert_eq!(spec["cpu"], "100m");
    assert_eq!(spec["memory"], "128Mi");
}

#[test]
fn test_parse_wasm_step() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "wasm_test" {
            steps {
                step "my-wasm" "Wasm" {
                    module = "gs://bucket/task.wasm"
                    function = "execute"
                    args = {
                        timeout = 60
                        retries = 3
                    }
                }
            }
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");
    let step = &workflow.steps[0];

    assert_eq!(step.r#type, "Wasm");
    let spec = &step.spec;
    assert_eq!(spec["module"], "gs://bucket/task.wasm");
    assert_eq!(spec["function"], "execute");
    assert_eq!(spec["args"]["timeout"], 60);
    assert_eq!(spec["args"]["retries"], 3);
}
