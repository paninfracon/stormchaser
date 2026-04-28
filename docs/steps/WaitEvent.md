# WaitEvent Step

The `WaitEvent` (also known as `Wait` or `WaitForEvent`) step pauses execution until a specific event is received by the system.

## DSL Specification

The `spec` block for `WaitEvent` supports the following fields:

* `correlation_key` (string, required): The field in the incoming event payload to match against.
* `correlation_value` (string, required): The expected value of the `correlation_key`.
* `timeout` (string, optional): Duration to wait for the event before timing out (e.g., "1h", "30m").

## Example

```hcl
step "wait_for_github" "WaitEvent" {
  spec = {
    correlation_key   = "pull_request.status"
    correlation_value = "merged"
    timeout           = "2h"
  }
}
```
