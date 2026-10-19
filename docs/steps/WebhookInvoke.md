# WebhookInvoke Step

The `WebhookInvoke` step sends an HTTP or HTTPS request, usually to notify another system. The engine runs it itself; no runner is involved. For calls that need a connection's credentials or outputs taken from the response, use [RestApi](RestApi.md).

Requests go through the engine's outbound guard: hosts on private and internal networks are refused unless the operator lists them in `STORMCHASER_SSRF_ALLOWED_HOSTS`, and redirects are not followed.

## DSL Specification

The `spec` block for `WebhookInvoke` supports the following fields:

* `url` (string, required): The endpoint to send the request to. It is not a Jinja template; use `${...}` to put values in it.
* `method` (string, optional): `POST` (the default), `GET`, `PUT`, `DELETE` or `PATCH`. Any other value is sent as `POST`.
* `headers` (object, optional): Key-value pairs of HTTP headers.
* `body` (string, optional): The request body, a MiniJinja template with `inputs`, `steps` (`steps.<name>.outputs.<key>`) and `run` (`run.id`) in its context.
* `timeout` (string, optional): How long to wait for a response, such as `"10s"`. Defaults to 30 seconds.

A 2xx response completes the step. Its body is kept as the step output `response`: the parsed JSON, or `{"text": "<body>"}` when it is not JSON. Any other status fails the step.

## Example

```hcl
step "notify_custom_api" "WebhookInvoke" {
  spec = {
    url    = "https://api.example.com/webhooks/deployments"
    method = "POST"
    headers = {
      "Content-Type"  = "application/json"
      "Authorization" = "Bearer ${secrets.API_TOKEN}"
    }
    body    = "{\"event\": \"deployment_started\", \"id\": \"{{ run.id }}\"}"
    timeout = "10s"
  }
}
```
