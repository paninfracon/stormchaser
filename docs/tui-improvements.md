# TUI Improvements

## Replacing ratatui-form (Completed)

We have successfully replaced `ratatui-form` as it was unmaintained and we had outgrown its capabilities. This transition was completed in two main areas:

### Area 1: Simple Forms (Completed)

`schemaui` has been deployed for forms with a fixed set of fields, known validation rules, and no need for dynamic data fetching (e.g., Storage Backends, Static Config).

Instead of custom Ratatui widget layouts, we now use static JSON Schema draft-07 files. When adding storage, the main loop suspends, hands the schema to the `schemaui` blocking runner, and waits for the validated `serde_json::Value`. This provides a zero-maintenance UI.

### Area 2: Workflow Inputs (Completed)

To handle SSE streams, external data fetches, and inter-field dependencies without blocking the UI, workflow inputs have been implemented as a **Reactive Dependency Graph** running within the Tokio event loop.

Fields act as nodes (`Loading`, `Ready`, `Resolved`), and the UI renders the current state of the graph. This prevents the terminal from locking up during asynchronous operations (like AWS list fetches) and ensures SSE connections remain stable. These forms are driven by an embedded HCL schema and input directives defined in the DSL.

## Upcoming Work

### Area 3: The HCL Editor (Pending)

**Goal:** Implement `ratatui-code-editor` in the TUI to provide syntax-highlighted editing of `.storm` files.

1. **Add Dependency:** Pull the `tree-sitter-hcl` crate into `Cargo.toml`.
2. **Grab Queries:** Navigate to the official `tree-sitter-hcl` GitHub repository's queries folder and copy the `highlights.scm` file. This file contains the Scheme-like syntax that maps AST nodes to highlight categories ("keywords", "strings", "variables").
3. **Inject at Initialization:** When instantiating the `Editor` struct, pass it the `tree-sitter-hcl::language()` function and the contents of `highlights.scm` using the custom highlights API.
