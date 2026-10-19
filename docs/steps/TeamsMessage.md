# TeamsMessage Step

The `TeamsMessage` step posts a message to a Microsoft Teams incoming webhook, as an Adaptive Card holding the text. The engine runs it itself; no runner is involved.

It needs an engine built with the `chatops-teams` feature (on by default). In a build without it the step fails, saying the feature is not enabled.

## DSL Specification

The `spec` block for `TeamsMessage` supports the following fields:

* `connection` (string, required): The name of an `http_api` connection whose `base_url` (or, in older connections, `url`) is the incoming webhook URL. The run's user must be allowed to use it.
* `message` (string, required): The text of the card. It is not a Jinja template; use `${...}` to put values in it.

A 2xx response completes the step; any other fails it. It has no outputs. The request goes through the engine's outbound guard, as [RestApi](RestApi.md) requests do.

## Example

```hcl
step "notify_teams" "TeamsMessage" {
  spec = {
    connection = "teams-ops"
    message    = "Run ${run.id} needs a decision."
  }
}
```
