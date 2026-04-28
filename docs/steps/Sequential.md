# Sequential Step

The `Sequential` step is a structural step that executes a nested block of child steps in order, one after the other.

## DSL Specification

The `Sequential` step does not use a `spec` block. Instead, it contains a nested `steps` block, defining all the steps that will be executed in sequence. If any step fails (and `allow_failure` is not true), the sequence will abort.

## Example

```hcl
step "build_and_deploy" "Sequential" {
  steps {
    step "build" "RunContainer" {
      spec = {
        image   = "docker:cli"
        command = ["docker", "build", "-t", "my-app", "."]
      }
    }
    step "deploy" "RunContainer" {
      spec = {
        image   = "my-deploy-tool"
        command = ["deploy", "--target", "prod"]
      }
    }
  }
}
```
