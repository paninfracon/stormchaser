# SlackMessage Step

The `SlackMessage` step posts a message to a Slack incoming webhook. The engine runs it itself; no runner is involved.

It needs an engine built with the `chatops-slack` feature (on by default). In a build without it the step fails, saying the feature is not enabled.

## DSL Specification

The `spec` block for `SlackMessage` supports the following fields:

* `connection` (string, required): The name of an `http_api` connection whose `base_url` (or, in older connections, `url`) is the incoming webhook URL. The run's user must be allowed to use it.
* `message` (string, required): The message text, sent as `text`. It is not a Jinja template; use `${...}` to put values in it.
* `blocks` (any value, optional): Slack Block Kit blocks, sent as `blocks`.

The step posts `{"text": ..., "blocks": ...}` as JSON. A 2xx response completes it; any other fails it. It has no outputs. The request goes through the engine's outbound guard, as [RestApi](RestApi.md) requests do.

## Example

```hcl
step "notify_slack" "SlackMessage" {
  spec = {
    connection = "slack-ops"
    message    = "Run ${run.id} deployed ${inputs.service} to production."
  }
}
```
