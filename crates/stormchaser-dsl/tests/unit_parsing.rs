use stormchaser_dsl::StormchaserParser;

#[test]
fn test_provision_new_syntax() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_provision" {
            storage "workspace" {
                size = "10Gi"
                provision {
                    download "my_script" {
                        url = "https://example.com/setup.sh"
                        destination = "scripts/setup.sh"
                        mode = "0755"
                    }
                    secret "docker_cfg" {
                        source = "secrets/docker"
                        destination = ".docker/config.json"
                    }
                }
            }
            steps {}
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");
    let storage = &workflow.storage[0];

    assert_eq!(storage.name, "workspace");
    assert_eq!(storage.provision.len(), 2);

    let download = &storage.provision[0];
    assert_eq!(download.name, "my_script");
    assert_eq!(download.resource_type, "download");
    assert_eq!(
        download.url.as_deref(),
        Some("https://example.com/setup.sh")
    );
    assert_eq!(download.destination, "scripts/setup.sh");
    assert_eq!(download.mode.as_deref(), Some("0755"));

    let secret = &storage.provision[1];
    assert_eq!(secret.name, "docker_cfg");
    assert_eq!(secret.resource_type, "secret");
    assert_eq!(secret.source.as_deref(), Some("secrets/docker"));
    assert_eq!(secret.destination, ".docker/config.json");
}

#[test]
fn test_provision_legacy_syntax() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_provision_legacy" {
            storage "workspace" {
                size = "5Gi"
                provision "my_artifact" {
                    resource_type = "artifact"
                    from = "other_workflow.workspace.output"
                    destination = "data/input"
                }
            }
            steps {}
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse legacy DSL");
    let storage = &workflow.storage[0];

    assert_eq!(storage.provision.len(), 1);
    let prov = &storage.provision[0];
    assert_eq!(prov.name, "my_artifact");
    assert_eq!(prov.resource_type, "artifact");
    assert_eq!(
        prov.from.as_deref(),
        Some("other_workflow.workspace.output")
    );
    assert_eq!(prov.destination, "data/input");
}

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
