# JinjaRender Step

The `JinjaRender` step renders a MiniJinja template and stores the text as a step output. Use it to prepare configuration files or messages, or to reshape data between steps. The engine runs it itself; no runner is involved.

## DSL Specification

The `spec` block for `JinjaRender` supports the following fields:

* `template` (string, required): The MiniJinja template to render.
* `context` (object, optional): Extra values for the template. Each key is added at the root of the context, and replaces a built-in value of the same name.
* `output_key` (string, optional): The name of the output that holds the result. Defaults to `result`.

### Template Context

* `inputs`: The run's inputs.
* `steps`: Outputs of steps that have finished, as `steps.<name>.outputs.<key>`.
* `run`: The run, as `run.id`.

Templates are rendered with MiniJinja 3, which follows Jinja2 closely: for example `tojson` writes `{"a": 1}` with a space after separators, `round` rounds half to even, and division by zero is an error.

A `${...}` in the template is resolved before the template is rendered, and its value goes in as text; see [`${...}` inside Jinja templates](../workflow_dsl.md#521--inside-jinja-templates).

If the engine rendering the template dies, the step is run again on another engine.

## Example

```hcl
step "generate_config" "JinjaRender" {
  spec = {
    template = <<EOT
server {
    listen 80;
    server_name {{ inputs.domain }};
    root {{ steps.build.outputs.path }};
}
EOT
    context = {
      extra_var = "some_value"
    }
    output_key = "nginx_config"
  }
}
```

A later step reads the result as `${steps.generate_config.outputs.nginx_config}`.
