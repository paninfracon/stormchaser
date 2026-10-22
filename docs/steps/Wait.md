# Wait Step

The `Wait` step pauses its run until an event names it by a correlation key and value. The step starts in the `waiting_for_event` state and is never sent to a runner. It is never retried.

## DSL Specification

The `spec` block for `Wait` supports the following fields:

* `correlation_key` (string, required): The key the event must carry.
* `correlation_value` (string, required): The value the event must carry. Usually an expression, such as `"${inputs.commit_sha}"`, resolved when the step is scheduled.
* `timeout` (string, optional): Stored but not enforced. A step left waiting ends only when its run times out.

## Resuming the step

Send `POST /api/v1/events/correlate` with a Stormchaser token and a JSON body whose `key` and `value` equal the step's `correlation_key` and `correlation_value`, compared as exact strings:

```json
{ "key": "github.commit.sha", "value": "3f2a9c1", "status": "merged" }
```

The step completes, and each top-level field of the body becomes one of its outputs (`${steps.<name>.outputs.status}`). The correlation is then removed, so a second event does not match. With no waiting step to match, the API answers `404`.

Webhook deliveries do not resume a `Wait` step, and `correlation_key` is not looked up as a path in an event's payload; the event has to be sent to the correlate endpoint in this form.

The step type is `Wait`. `WaitEvent`, a name earlier versions of this page used, is not a step type the engine knows, and a step of that type is never run.

## Example

```hcl
step "wait_for_merge" "Wait" {
  spec = {
    correlation_key   = "github.commit.sha"
    correlation_value = "${inputs.commit_sha}"
  }
  next = ["deploy"]
}
```
