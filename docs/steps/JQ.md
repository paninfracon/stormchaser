# JQ Step

The `JQ` step transforms JSON with a `jq` program. It runs in one of two ways, depending on its spec:

* **Inline.** With neither `input_file` nor `output_file`, the engine evaluates the program itself on `input`, with [jaq](https://github.com/01mf02/jaq) 3, and stores the result in the step output `result`.
* **Files.** With `input_file` or `output_file`, the step becomes a `RunContainer` step that runs `jq -c` in the `ghcr.io/jqlang/jq:latest` image on a runner. `input` is then ignored, and the step has no `result` output: the program's output goes to `output_file`, or to the step's log if there is none. The files are on the storage the step mounts with `storage_mounts`.

## DSL Specification

The `spec` block for `JQ` supports the following fields:

* `program` (string, required): The `jq` program.
* `input` (any value, optional): The JSON value the program reads, inline mode only. Defaults to `null`.
* `input_file` (string, optional): Path of a JSON file to read, in the container. Switches to file mode.
* `output_file` (string, optional): Path to write the result to, in the container. Switches to file mode.
* `storage_mounts` (list of objects, optional): Storage to mount in file mode, as for [RunContainer](RunContainer.md).

### Outputs (inline mode)

* `result`: The program's output. A program that produces one value gives that value; one that produces several gives a list of them.

A program that raises an error or calls `halt` or `halt_error` fails the step.

## Example (inline)

```hcl
step "transform" "JQ" {
  spec = {
    program = ".items | map(.name)"
    input = {
      items = [
        { name = "foo", val = 1 },
        { name = "bar", val = 2 }
      ]
    }
  }
}
```

A later step reads the list as `${steps.transform.outputs.result}`.

## Example (files)

```hcl
step "transform_file" "JQ" {
  spec = {
    program        = ".version"
    input_file     = "/workspace/package.json"
    output_file    = "/workspace/version.json"
    storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
  }
}
```

## Limits

In inline mode the program runs in a separate process with no access to the engine's environment (the `env` function and `$ENV` are not available). It is stopped after 10 seconds (`STORMCHASER_JQ_TIMEOUT_SECS`) or if it uses more than 512 MiB (`STORMCHASER_JQ_MAX_MEMORY_MB`), and it may return at most 10 MiB. A program that is stopped fails the step. In file mode the step is a container step and has a container step's limits instead (see [Limits](../workflow_dsl.md#48-step-limits)).

If the engine running an inline JQ step dies, the step is run again on another engine, since it has no effect but its output.
