use std::collections::HashMap;

use anyhow::{Context, Result};
use hcl::{Block, Body};
use serde_json::{Map, Value};
use stormchaser_model::dsl;

use crate::ast::Step;

use super::expr::{block_to_value, expr_to_string, expr_to_value};
use super::provision::{parse_provision_legacy_block, parse_provision_sub_block};
use super::step::parse_steps;

pub struct ParsedWorkflowBlocks {
    pub steps: Vec<Step>,
    pub storage: Vec<dsl::Storage>,
    pub inputs: Vec<dsl::Input>,
    pub outputs: Vec<dsl::Output>,
    pub step_libraries: Vec<dsl::StepLibrary>,
    pub includes: Vec<dsl::Include>,
    pub inputs_schema: Option<Value>,
    pub queries: Vec<dsl::Query>,
    pub inputs_view: Option<dsl::InputView>,
    pub libraries: Vec<dsl::Library>,
    pub strategy: Option<dsl::Strategy>,
    pub quotas: Option<dsl::Quotas>,
    pub handlers: Vec<dsl::Handler>,
    pub on_failure: Option<Vec<Step>>,
    pub finally: Option<Vec<Step>>,
    pub aliases: HashMap<String, String>,
}

pub fn parse_workflow_blocks(body: &Body) -> Result<ParsedWorkflowBlocks> {
    let mut steps = Vec::new();
    let mut storage = Vec::new();
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut step_libraries = Vec::new();
    let mut includes = Vec::new();
    let mut inputs_schema = None;
    let mut queries = Vec::new();
    let mut inputs_view = None;
    let mut libraries = Vec::new();
    let mut strategy = None;
    let mut quotas = None;
    let mut handlers = Vec::new();
    let mut on_failure = None;
    let mut finally = None;
    let mut aliases = HashMap::new();

    for block in body.blocks() {
        match block.identifier() {
            "steps" => {
                steps = parse_steps(block.body())?;
            }
            "storage" => {
                storage.push(parse_storage_block(block)?);
            }
            "input" => {
                inputs.push(parse_input_block(block)?);
            }
            "output" => {
                outputs.push(parse_output_block(block)?);
            }
            "step_library" => {
                step_libraries.push(parse_step_library_block(block)?);
            }
            "include" => {
                includes.push(parse_include_block(block)?);
            }
            "query" => {
                queries.push(parse_query_block(block)?);
            }
            "inputs" if block.labels().is_empty() => {
                let mut schema = crate::hcl_schema::hcl_to_json_schema(block.body())?;
                if let Value::Object(ref mut map) = schema {
                    if let Some(Value::Array(arr)) = map.remove("ui_order") {
                        let ui_order: Vec<String> = arr
                            .into_iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        inputs_view = Some(dsl::InputView { ui_order });
                    }
                }
                inputs_schema = Some(schema);
            }
            "library" => {
                libraries.push(parse_library_block(block)?);
            }
            "strategy" => {
                strategy = Some(parse_strategy_block(block)?);
            }
            "quotas" => {
                quotas = Some(parse_quotas_block(block)?);
            }
            "handler" => {
                handlers.push(parse_handler_block(block)?);
            }
            "on_failure" => {
                on_failure = Some(parse_steps(block.body())?);
            }
            "finally" => {
                finally = Some(parse_steps(block.body())?);
            }
            "aliases" => {
                for attr in block.body().attributes() {
                    aliases.insert(attr.key().to_string(), expr_to_string(attr.expr())?);
                }
            }
            _ => {}
        }
    }

    Ok(ParsedWorkflowBlocks {
        steps,
        storage,
        inputs,
        outputs,
        step_libraries,
        includes,
        inputs_schema,
        queries,
        inputs_view,
        libraries,
        strategy,
        quotas,
        handlers,
        on_failure,
        finally,
        aliases,
    })
}

fn parse_query_block(block: &Block) -> Result<dsl::Query> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Query block must have a name label")?;
    let mut r#type = String::new();
    let mut params = HashMap::new();

    for attr in block.body().attributes() {
        match attr.key() {
            "type" => r#type = expr_to_string(attr.expr())?,
            "params" => {
                if let Value::Object(obj) = expr_to_value(attr.expr())? {
                    for (k, v) in obj {
                        if let Value::String(s) = v {
                            params.insert(k, s);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(dsl::Query {
        name,
        r#type,
        params,
    })
}

fn parse_storage_block(block: &Block) -> Result<dsl::Storage> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Storage block must have a name label")?;
    let mut size = "1Gi".to_string();
    let mut backend = None;
    let mut provision = Vec::new();
    let mut artifacts = Vec::new();

    for attr in block.body().attributes() {
        if attr.key() == "size" {
            size = expr_to_string(attr.expr())?;
        } else if attr.key() == "backend" {
            backend = Some(expr_to_string(attr.expr())?);
        }
    }

    for inner in block.body().blocks() {
        if inner.identifier() == "artifact" {
            let art_name = inner
                .labels()
                .first()
                .map(|l| l.as_str().to_string())
                .context("Artifact block must have a name label")?;
            let mut path = String::new();
            let mut retention = "on_success".to_string();
            for attr in inner.body().attributes() {
                if attr.key() == "path" {
                    path = expr_to_string(attr.expr())?;
                } else if attr.key() == "retention" {
                    retention = expr_to_string(attr.expr())?;
                }
            }
            artifacts.push(dsl::Artifact {
                name: art_name,
                path,
                retention,
            });
        } else if inner.identifier() == "provision" {
            if inner.labels().is_empty() {
                for prov_block in inner.body().blocks() {
                    provision.push(parse_provision_sub_block(prov_block)?);
                }
            } else {
                provision.push(parse_provision_legacy_block(inner)?);
            }
        }
    }

    Ok(dsl::Storage {
        name,
        backend,
        size,
        provision,
        preserve: vec![],
        artifacts,
        retainment: None,
    })
}

fn parse_input_block(block: &Block) -> Result<dsl::Input> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Input block must have a name label")?;
    let mut r#type = "string".to_string();
    let mut default = None;
    let mut description = None;
    let mut validation = None;
    let mut options = None;
    for attr in block.body().attributes() {
        if attr.key() == "type" {
            r#type = expr_to_string(attr.expr())?;
        } else if attr.key() == "default" {
            default = Some(expr_to_value(attr.expr())?);
        } else if attr.key() == "description" {
            description = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "validation" {
            validation = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "options" {
            if let Value::Array(arr) = expr_to_value(attr.expr())? {
                let mut opts = Vec::new();
                for v in arr {
                    if let Value::String(s) = v {
                        opts.push(s);
                    }
                }
                options = Some(opts);
            }
        }
    }
    Ok(dsl::Input {
        name,
        r#type,
        description,
        default,
        validation,
        options,
    })
}

fn parse_output_block(block: &Block) -> Result<dsl::Output> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("Output block must have a name label")?;
    let mut value = String::new();
    for attr in block.body().attributes() {
        if attr.key() == "value" {
            value = expr_to_string(attr.expr())?;
        }
    }
    Ok(dsl::Output { name, value })
}

fn parse_step_library_block(block: &Block) -> Result<dsl::StepLibrary> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("step_library block must have a name label")?;
    let mut r#type = String::new();
    let mut params = HashMap::new();
    let mut spec_map = Map::new();
    let mut timeout = None;
    let mut allow_failure = None;

    for attr in block.body().attributes() {
        match attr.key() {
            "type" => r#type = expr_to_string(attr.expr())?,
            "params" => {
                if let Value::Object(obj) = expr_to_value(attr.expr())? {
                    for (k, v) in obj {
                        if let Value::String(s) = v {
                            params.insert(k, s);
                        }
                    }
                }
            }
            "timeout" => timeout = Some(expr_to_string(attr.expr())?),
            "allow_failure" => {
                if let Value::Bool(b) = expr_to_value(attr.expr())? {
                    allow_failure = Some(b);
                }
            }
            _ => {}
        }
    }

    for inner in block.body().blocks() {
        if inner.identifier() == "spec" {
            for attr in inner.body().attributes() {
                spec_map.insert(attr.key().to_string(), expr_to_value(attr.expr())?);
            }
            for inner_block in inner.body().blocks() {
                let key = inner_block.identifier().to_string();
                let value = block_to_value(inner_block)?;
                spec_map.insert(key, value);
            }
        } else if inner.identifier() == "params" {
            for attr in inner.body().attributes() {
                params.insert(attr.key().to_string(), expr_to_string(attr.expr())?);
            }
        }
    }

    Ok(dsl::StepLibrary {
        name,
        r#type,
        params,
        spec: Value::Object(spec_map),
        timeout,
        allow_failure,
        retry: None,
    })
}

fn parse_include_block(block: &Block) -> Result<dsl::Include> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("include block must have a name label")?;
    let mut workflow = String::new();
    let mut inputs_map = HashMap::new();

    for attr in block.body().attributes() {
        if attr.key() == "workflow" {
            workflow = expr_to_string(attr.expr())?;
        } else if attr.key() == "inputs" {
            if let Value::Object(obj) = expr_to_value(attr.expr())? {
                for (k, v) in obj {
                    if let Value::String(s) = v {
                        inputs_map.insert(k, s);
                    } else {
                        inputs_map.insert(k, v.to_string());
                    }
                }
            }
        }
    }

    Ok(dsl::Include {
        name,
        workflow,
        inputs: inputs_map,
    })
}

fn parse_library_block(block: &Block) -> Result<dsl::Library> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("library block must have a name label")?;
    let mut source = String::new();
    let mut version = String::new();
    let mut checksum = String::new();

    for attr in block.body().attributes() {
        if attr.key() == "source" {
            source = expr_to_string(attr.expr())?;
        } else if attr.key() == "version" {
            version = expr_to_string(attr.expr())?;
        } else if attr.key() == "checksum" {
            checksum = expr_to_string(attr.expr())?;
        }
    }

    Ok(dsl::Library {
        name,
        source,
        version,
        checksum,
    })
}

fn parse_strategy_block(block: &Block) -> Result<dsl::Strategy> {
    let mut affinity = None;
    let mut fail_fast = None;
    let mut max_parallel = None;
    let mut process_allow_list = None;

    for attr in block.body().attributes() {
        if attr.key() == "affinity" {
            affinity = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "fail_fast" {
            if let Value::Bool(b) = expr_to_value(attr.expr())? {
                fail_fast = Some(b);
            }
        } else if attr.key() == "max_parallel" {
            if let Value::Number(n) = expr_to_value(attr.expr())? {
                if let Some(num) = n.as_u64() {
                    max_parallel = Some(num as u32);
                }
            }
        } else if attr.key() == "process_allow_list" {
            if let Value::Array(arr) = expr_to_value(attr.expr())? {
                let mut list = Vec::new();
                for v in arr {
                    if let Value::String(s) = v {
                        list.push(s);
                    }
                }
                process_allow_list = Some(list);
            }
        }
    }

    Ok(dsl::Strategy {
        affinity,
        fail_fast,
        max_parallel,
        process_allow_list,
    })
}

fn parse_quotas_block(block: &Block) -> Result<dsl::Quotas> {
    let mut max_concurrency = None;
    let mut max_cpu = None;
    let mut max_memory = None;
    let mut max_storage = None;
    let mut timeout = None;

    for attr in block.body().attributes() {
        if attr.key() == "max_concurrency" {
            if let Value::Number(n) = expr_to_value(attr.expr())? {
                if let Some(num) = n.as_u64() {
                    max_concurrency = Some(num as u32);
                }
            }
        } else if attr.key() == "max_cpu" {
            max_cpu = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "max_memory" {
            max_memory = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "max_storage" {
            max_storage = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "timeout" {
            timeout = Some(expr_to_string(attr.expr())?);
        }
    }

    Ok(dsl::Quotas {
        max_concurrency,
        max_cpu,
        max_memory,
        max_storage,
        timeout,
    })
}

fn parse_handler_block(block: &Block) -> Result<dsl::Handler> {
    let name = block
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .context("handler block must have a name label")?;
    let mut event_type = String::new();
    let mut condition = None;
    let mut action = String::new();

    for attr in block.body().attributes() {
        if attr.key() == "event_type" {
            event_type = expr_to_string(attr.expr())?;
        } else if attr.key() == "condition" {
            condition = Some(expr_to_string(attr.expr())?);
        } else if attr.key() == "action" {
            action = expr_to_string(attr.expr())?;
        }
    }

    Ok(dsl::Handler {
        name,
        event_type,
        condition,
        action,
    })
}
