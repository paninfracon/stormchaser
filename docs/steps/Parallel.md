# Parallel Step

The `Parallel` step is a structural step that executes a nested block of child steps concurrently.

## DSL Specification

The `Parallel` step does not use a `spec` block. Instead, it contains a nested `steps` block, defining all the steps that will be executed at the same time. The parallel step will only complete when all of its child steps have completed.

## Example

```hcl
step "run_tests" "Parallel" {
  steps {
    step "test_backend" "RunContainer" {
      spec = {
        image   = "golang:1.21"
        command = ["go", "test", "./..."]
      }
    }
    step "test_frontend" "RunContainer" {
      spec = {
        image   = "node:18"
        command = ["npm", "run", "test"]
      }
    }
  }
}
```
