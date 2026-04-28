# Wasm Step

The `Wasm` step executes a WebAssembly module. This provides a fast, lightweight, and secure way to execute custom logic without container overhead.

## DSL Specification

The `spec` block for `Wasm` supports the following fields:

* `module` (string, required): URI to the WASM module. Can be a local path (`file://`), an S3 URI (`s3://`), or a registry name.
* `function` (string, required): The name of the function to export and execute from the WebAssembly module.
* `args` (object, optional): JSON arguments to pass into the function.

## Example

```hcl
step "process_data" "Wasm" {
  spec = {
    module   = "file://artifacts/data_processor.wasm"
    function = "transform_payload"
    args = {
      input_format  = "csv"
      output_format = "json"
    }
  }
}
```
