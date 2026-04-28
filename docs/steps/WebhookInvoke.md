# WebhookInvoke Step

The `WebhookInvoke` step makes an HTTP/HTTPS call to an external service. This is useful for triggering external APIs or notifying other systems.

## DSL Specification

The `spec` block for `WebhookInvoke` supports the following fields:

* `url` (string, required): The endpoint to send the request to.
* `method` (string, optional): The HTTP method to use (e.g., "GET", "POST", "PUT"). Defaults to "POST".
* `headers` (object, optional): Key-value pairs of HTTP headers.
* `body` (string, optional): The request body. Supports MiniJinja template rendering.
* `timeout` (string, optional): Duration to wait for a response (e.g., "30s", "1m").

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
    body    = "{\"event\": \"deployment_started\", \"id\": \"{{ run_id }}\"}"
    timeout = "10s"
  }
}
```
