# Wasm Step

The `Wasm` step runs a function of a WebAssembly module inside the engine. It suits small pieces of logic that do not need a container. The engine runs it itself; no runner is involved.

## DSL Specification

The `spec` block for `Wasm` supports the following fields:

* `module` (string, required): Where to load the module from: an `http://` or `https://` URL, or `base64://` followed by the encoded module. Local paths are not accepted.
* `function` (string, optional): The exported function to call. Defaults to `run`.
* `args` (any value, optional): Data for the function. It reaches the module as part of the spec (see below).

## What the module receives and returns

The module must export `memory`, `alloc(len: i32) -> i32`, `dealloc(ptr: i32, len: i32)`, `get_result_len() -> i32` and the function itself, `(ptr: i32, len: i32) -> i32`. The engine writes a JSON document into memory that `alloc` reserves and calls the function with its address and length:

```json
{ "spec": { "module": "...", "function": "...", "args": { } }, "params": { }, "inputs": { }, "config": null }
```

`spec` is the step's resolved spec, `params` its params and `inputs` the run's inputs. The function returns the address of a JSON result, whose length `get_result_len` gives. If the result is an object, each of its keys becomes a step output; any other result sets no outputs.

## Registered step types

A WASM module can also be registered as a step type of its own, by publishing to the NATS subject `stormchaser.v1.step.register_wasm` with `step_type`, `wasm_module`, and optionally `wasm_function` (default `run`), `wasm_config`, `schema` and `documentation`. A step of that type then runs the module as above, with `wasm_config` as `config`. A registration cannot take the name of a built-in step type.

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
| Modules running at once | 4 | `STORMCHASER_WASM_MAX_CONCURRENT` |

A module that exceeds a limit fails the step; one that has to wait for a free slot starts its time limit when it gets one. The settings are environment variables on the engine and apply to every module, including registered step definitions.
