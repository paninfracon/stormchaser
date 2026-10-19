# Sequential Step (Planned)

`Sequential` is planned as a group step that runs a nested `steps` block of child steps in order, one after the other. It is not implemented.

The parser accepts a step with a nested `steps` block and stores the children, but the engine never schedules them. When a `Sequential` step is dispatched it fails with "Sequential steps are not supported yet", and its run fails unless the step sets `allow_failure`.

## What works today

Order steps with `next`. A step starts when every step that lists it in `next` has finished, and a failed step fails the run unless it sets `allow_failure`, so later steps do not start. See [Ordering steps](../workflow_dsl.md#452-ordering-steps).

```hcl
step "build" "RunContainer" {
  spec = {
    image   = "docker:cli"
    command = ["docker", "build", "-t", "my-app", "."]
  }
  next = ["deploy"]
}

step "deploy" "RunContainer" {
  spec = {
    image   = "my-deploy-tool"
    command = ["deploy", "--target", "prod"]
  }
}
```
