# Plan: Workspace Input Schemas

This is a complete plan to implement workspace input schemas using the existing HCL embedded schema format. This approach leverages the `hcl_to_json_schema` parser already present in the DSL, validates payloads using the `jsonschema` crate, integrates with the `schemaui` crate for the TUI, and introduces dynamic query capabilities.

## Phase 1: Model Updates (`stormchaser-model`)

Currently, workflow inputs are stored as a `Vec<Input>` (parsed from multiple `input "name" { ... }` blocks). We will expand the AST to support a consolidated `inputs_schema` definition.

1. **Update `Workflow` AST (`crates/stormchaser-model/src/dsl/workflow.rs`)**:
   Add a new field to hold the compiled JSON schema object.

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
   pub struct Workflow {
       // ... existing fields ...
       #[serde(default)]
       pub inputs_schema: Option<serde_json::Value>,
   }
   ```

   *Note: We can preserve `pub inputs: Vec<Input>` to maintain backward compatibility with older `input` blocks or eventually deprecate them.*

2. **Schema Extension for Dynamic Queries**:
   In order to support Rundeck-style dynamic dropdowns, we will allow custom attributes like `query` or `options_query` alongside standard schema constraints. Since JSON Schema allows unknown keywords, these will naturally be preserved in the `serde_json::Value` mapping and can be extracted later by the Engine and TUI.

## Phase 2: DSL Parser Updates (`stormchaser-dsl`)

We will teach the parser to recognize an `inputs` block (with no labels) and pass its body to the existing HCL-to-JSON Schema converter.

1. **Modify `parse_workflow_block` (`crates/stormchaser-dsl/src/parser.rs`)**:
   When iterating over the `Workflow` block's children, detect `inputs` blocks that lack string labels:

   ```rust
   // Inside the block processing loop for workflows
   } else if block.identifier() == "inputs" && block.labels().is_empty() {
       let schema = crate::hcl_schema::hcl_to_json_schema(block.body())?;
       inputs_schema = Some(schema);
   }
   ```

## Phase 3: Engine Validation Logic (`stormchaser-engine`)

We will intercept workflow executions and validate incoming payloads against the `inputs_schema`.

1. **Schema Validation (`crates/stormchaser-engine/src/workflow_machine.rs`)**:
   In the workflow start/queueing sequence, inject validation logic.

   ```rust
   if let Some(schema_val) = &workflow.inputs_schema {
       let compiled_schema = jsonschema::JSONSchema::compile(schema_val)
           .map_err(|e| anyhow::anyhow!("Failed to compile input schema: {}", e))?;

       let inputs_json = serde_json::to_value(&runtime_inputs)?;
       if let Err(errors) = compiled_schema.validate(&inputs_json) {
           let err_msg = errors
               .map(|e| e.to_string())
               .collect::<Vec<_>>()
               .join(", ");
           return Err(anyhow::anyhow!("Input validation failed: {}", err_msg));
       }
   }
   ```

2. **Default Value Hydration**:
   JSON Schema does not automatically inject `default` values into payloads during validation. A pre-processing step will iterate over `inputs_schema["properties"]` and assign any defined `default` values to missing keys in the `runtime_inputs` payload.

## Phase 4: Dynamic Options via Queries (API / SQL / AWS)

Similar to Rundeck, we need the ability to populate dynamic dropdowns or validation data using external queries.

1. **Query Resolution Context**:
   When an input field contains a custom `query` property (e.g. `query = "SELECT id, name FROM envs"`, `query = "api://my-service/options"`, or `query = "aws://ec2/describe-instances"`):
   - The engine (or API endpoint requested by the UI) will parse the `query` string.
   - We will implement a `QueryResolver` in `stormchaser-engine` or `stormchaser-api` that interprets the protocol (e.g. `sql://`, `api://`, `aws://`, or standard internal queries) and executes the request securely. For AWS, it should integrate with the `aws-config` and `aws-sdk-*` crates we already use to perform authenticated calls against the AWS API.
2. **Schema Hydration Endpoint**:
   Create a new API route in `stormchaser-api` (e.g. `GET /api/v1/workflows/:id/schema/hydrated`) that executes these queries dynamically and rewrites the returned `inputs_schema` `enum` arrays with the fetched results. This ensures UIs don't have to perform the queries directly.

## Phase 5: TUI Integration with `schemaui` (`stormchaser-tui`)

We will replace basic text boxes in the TUI workflow start screen with intelligent, schema-driven forms.

1. **Include Dependency**: Add the `schemaui` crate to the `stormchaser-tui` `Cargo.toml`.
2. **Render the Form**:
   When prompting the user for workflow inputs in the TUI, retrieve the `inputs_schema` (ideally via the hydrated API endpoint mentioned in Phase 4 so dynamic queries are resolved).
3. **Map Schema to TUI Controls**:
   Use `schemaui` to dynamically generate the form elements:
   - Map `type: "string"` and `format: "email"` to specialized input boxes.
   - Map `enum` lists to Dropdowns/Select lists.
   - Handle conditional rendering (e.g., `oneOf`, `anyOf`, or `dependencies` if supported by `schemaui`).
   - Run local validation against the user's input before submission to provide immediate visual feedback.

## Phase 6: Testing & Documentation

1. **DSL Unit Tests (`stormchaser-dsl`)**:
   Assert that custom query properties compile correctly:

   ```hcl
   inputs {
     type = "object"
     properties {
       environment = string(query("sql://db/environments"))
     }
   }
   ```

2. **Engine & UI Integration Tests**:
   Create `schema-validation.storm` and `dynamic-query.storm` execution tests.
3. **Documentation Updates**:
   Update `docs/workflow_dsl.md` to showcase the new `inputs` schema format, TUI `schemaui` integration, and Rundeck-style dynamic queries.
