# Parallel Step (Planned)

`Parallel` is planned as a group step that runs a nested `steps` block of child steps at the same time, and completes when they all have. It is not implemented.

The parser accepts a step with a nested `steps` block and stores the children, but the engine never schedules them, and no runner or engine handler takes a step of type `Parallel`. A run that reaches one waits until it times out.

## What works today

Steps already run in parallel unless `next` orders them: every step that is not in another step's `next` list starts when the run starts, and a step listed in several `next` lists starts when all of those steps have finished. See [Ordering steps](../workflow_dsl.md#452-ordering-steps).

To run one step over many items at once, give it `iterate` (see [Iteration](../workflow_dsl.md#46-iteration)):

```hcl
step "test" "RunContainer" {
  iterate = ["backend", "frontend"]
  as      = "component"
  strategy {
    max_parallel = 2
  }
  spec = {
    image   = "my-test-runner:latest"
    command = ["run-tests", "${component}"]
  }
}
```
