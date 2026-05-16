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

#[test]
fn test_parse_inputs_view() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_view" {
            inputs {
                ui_order = ["field_b", "field_a", "*"]
                type = "object"
                properties = {
                    field_a = { type = "string" }
                    field_b = { type = "string" }
                }
            }
            steps {}
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");

    assert!(workflow.inputs_view.is_some());
    let view = workflow.inputs_view.unwrap();
    assert_eq!(view.ui_order, vec!["field_b", "field_a", "*"]);
}

#[test]
fn test_parse_include_library_strategy_quotas_handler() {
    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "test_blocks" {
            include "my_include" {
                workflow = "other_wf"
                inputs = {
                    foo = "bar"
                    baz = 123
                }
            }
            library "my_lib" {
                source = "git://example.com/repo"
                version = "v1.0.0"
                checksum = "sha256:1234"
            }
            strategy {
                affinity = "zone-a"
                fail_fast = true
                max_parallel = 5
                process_allow_list = ["safe_process", "other"]
            }
            quotas {
                max_concurrency = 10
                max_cpu = "2"
                max_memory = "4Gi"
                max_storage = "10Gi"
                timeout = "1h"
            }
            handler "on_fail" {
                event_type = "failed"
                condition = "true"
                action = "notify"
            }
            steps {}
        }
    "#;

    let parser = StormchaserParser::new();
    let workflow = parser.parse(dsl).expect("Failed to parse DSL");

    assert_eq!(workflow.includes.len(), 1);
    let inc = &workflow.includes[0];
    assert_eq!(inc.name, "my_include");
    assert_eq!(inc.workflow, "other_wf");
    assert_eq!(inc.inputs.get("foo").unwrap(), "bar");
    assert_eq!(inc.inputs.get("baz").unwrap(), "123");

    assert_eq!(workflow.libraries.len(), 1);
    let lib = &workflow.libraries[0];
    assert_eq!(lib.name, "my_lib");
    assert_eq!(lib.source, "git://example.com/repo");
    assert_eq!(lib.version, "v1.0.0");
    assert_eq!(lib.checksum, "sha256:1234");

    let strat = workflow.strategy.as_ref().unwrap();
    assert_eq!(strat.affinity.as_deref(), Some("zone-a"));
    assert_eq!(strat.fail_fast, Some(true));
    assert_eq!(strat.max_parallel, Some(5));
    assert_eq!(
        strat.process_allow_list.as_ref().unwrap(),
        &vec!["safe_process".to_string(), "other".to_string()]
    );

    let quotas = workflow.quotas.as_ref().unwrap();
    assert_eq!(quotas.max_concurrency, Some(10));
    assert_eq!(quotas.max_cpu.as_deref(), Some("2"));
    assert_eq!(quotas.max_memory.as_deref(), Some("4Gi"));
    assert_eq!(quotas.max_storage.as_deref(), Some("10Gi"));
    assert_eq!(quotas.timeout.as_deref(), Some("1h"));

    assert_eq!(workflow.handlers.len(), 1);
    let handler = &workflow.handlers[0];
    assert_eq!(handler.name, "on_fail");
    assert_eq!(handler.event_type, "failed");
    assert_eq!(handler.condition.as_deref(), Some("true"));
    assert_eq!(handler.action, "notify");
}
