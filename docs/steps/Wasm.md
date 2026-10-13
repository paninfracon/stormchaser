# Wasm Step

The `Wasm` step executes a WebAssembly module. This provides a fast, lightweight, and secure way to execute custom logic without container overhead.

## DSL Specification

The `spec` block for `Wasm` supports the following fields:

* `module` (string, required): Where to load the WASM module from: an `http://` or `https://` URL, or `base64://` followed by the encoded module. Local paths are not accepted.
* `function` (string, optional): The name of the exported function to call. Defaults to `run`.
* `args` (object, optional): JSON arguments to pass into the function.

## Example

```hcl
step "process_data" "Wasm" {
  spec = {
    module   = "https://artifacts.example.com/data_processor.wasm"
    function = "transform_payload"
    args = {
      input_format  = "csv"
      output_format = "json"
    }
  }
}
```

## Limits

A module runs inside the engine process, with no access to the host: it has no imports, so it cannot open files or sockets. It is also bounded:

| Limit | Default | Setting |
|---|---|---|
| Time to download, compile and run | 30 seconds | `STORMCHASER_WASM_TIMEOUT_SECS` |
| Linear memory | 256 MiB | `STORMCHASER_WASM_MAX_MEMORY_MB` |
| Module size | 64 MiB | fixed |
| Result size | 10 MiB | fixed |

A module that exceeds a limit fails the step. Both settings are environment variables on the engine and apply to every module, including registered step definitions.
