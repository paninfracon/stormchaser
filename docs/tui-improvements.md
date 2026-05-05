# Replacing ratatui-form

We need to replace ratatui-form as both it is unmaintained and we have outgrown the capabilities

## Area 1: Simple Forms (Storage Backends, Static Config)

This is where you deploy `schemaui`. You have a fixed set of fields, known validation rules, and zero need for dynamic data fetching during the input process.

Instead of writing custom Ratatui widget layouts for every new storage backend or integration you add, you just write a static JSON Schema draft-07 file. When the user hits "Add Storage," your main loop suspends, hands the schema to the `schemaui` blocking runner, and waits for the validated `serde_json::Value` to come back. It’s zero-maintenance UI.

### Area 2: Workflow Inputs (The Reactive DAG)

This is the core of the engine's execution UX and where you abandon static schemas. Because you have SSE streams, AWS list fetches, and inter-field dependencies, this must be built as a **Reactive Dependency Graph** running entirely inside your asynchronous Tokio loop.

You define your fields as nodes (`Loading`, `Ready`, `Resolved`). The UI is completely dumb—it just renders whatever state the graph is currently in on every tick. If an AWS fetch takes two seconds, the UI thread doesn't care; it just draws a spinner for that specific node while the user continues typing into a regex-validated text field on another node. This guarantees the terminal never locks up and your SSE connections never drop.

This set of forms will be defined by a modified JSON schema using embedded HCL schema (which we already support converting to and from json schema) and a combination of input directives in the DSL
DSL will support a domain specific and limited form of schema with the ability to fetch data from external sources for validation and dropdowns (a la Rundeck)

## Area 3: The HCL Editor

Using ratatui-code-editor in the TUI

Add the Dependency: Pull the tree-sitter-hcl crate into your Cargo.toml.

Grab the Queries: Go to the official tree-sitter-hcl GitHub repository, navigate to their queries folder, and copy the highlights.scm file. This file contains the Scheme-like syntax that tells Tree-sitter which AST nodes are "keywords," "strings," or "variables."

Inject at Initialization: When you instantiate the Editor struct in your application, you will pass it the tree-sitter-hcl::language() function and the contents of that highlights.scm file using the custom highlights API.
