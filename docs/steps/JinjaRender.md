# JinjaRender Step

The `JinjaRender` step allows for flexible text generation using the MiniJinja templating engine. This is useful for preparing configuration files, generating custom notifications, or transforming data between steps.

## DSL Specification

The `spec` block for `JinjaRender` supports the following fields:

* `template` (string, required): The MiniJinja template string to render.
* `context` (object, optional): Additional key-value pairs to include in the template rendering context. These will be merged with the global context.
* `output_key` (string, optional): The key under which the rendered result will be stored in the step's outputs. Defaults to "result".

### Template Context

The following objects are automatically available in the template context:

* `inputs`: The workflow workflow inputs.
* `steps`: Outputs from previous steps.
* `run`: Metadata about the current run (e.g., `run.id`).

Any values provided in the `context` field of the spec will be merged directly into the root of the template context.

## Example

```hcl
step "generate_config" "JinjaRender" {
  spec = {
    template = <<EOT
server {
    listen 80;
    server_name {{ inputs.domain }};
    root {{ steps.build.path }};
}
EOT
    context = {
      "extra_var" = "some_value"
    }
    output_key = "nginx_config"
  }
}
```

The output can then be accessed in subsequent steps using `${steps.generate_config.nginx_config}`.
