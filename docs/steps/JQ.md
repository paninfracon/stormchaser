# JQ Step

The `JQ` step processes JSON input using a native Rust implementation of the popular `jq` tool. It transforms and filters JSON data dynamically.

## DSL Specification

The `spec` block for `JQ` supports the following fields:

* `program` (string, required): The JQ query/program to execute.
* `input` (object, optional): The JSON data to process as an inline variable or object.
* `input_file` (string, optional): Path to a JSON file to read the input from. If provided, overrides `input`.
* `output_file` (string, optional): Path to write the resulting JSON output to.

*Note: You must provide either `input` or `input_file`.*

Programs are evaluated with [jaq](https://github.com/01mf02/jaq) 3. A program that raises an error or calls `halt`/`halt_error` fails the step.

## Example (Inline Variables)

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
  outputs = [
    { name = "jq_result", source = "result" }
  ]
}
```

## Example (File-based)

```hcl
step "transform_file" "JQ" {
  spec = {
    program = ".version"
    input_file = "package.json"
    output_file = "version.json"
  }
}
```

## Limits

A program runs in a separate process with no access to the engine's environment
(the `env` function and `$ENV` are not available). It is stopped after 10
seconds (`STORMCHASER_JQ_TIMEOUT_SECS`) or if it uses more than 512 MiB
(`STORMCHASER_JQ_MAX_MEMORY_MB`), and it may return at most 10 MiB. A program
that is stopped fails the step.
