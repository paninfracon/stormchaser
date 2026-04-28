# RunContainer Step

The `RunContainer` step executes a Docker container. This is the most common step type for general-purpose scripting, building, and testing tasks.

## DSL Specification

The `spec` block for `RunContainer` maps to the `CommonContainerSpec` and supports the following fields:

* `image` (string, required): The Docker image to run.
* `command` (list of strings, optional): Overrides the default entrypoint of the image.
* `args` (list of strings, optional): Arguments to pass to the command.
* `env` (list of objects, optional): Environment variables to set in the container.
* `cpu` (string, optional): CPU limit (e.g., "500m").
* `memory` (string, optional): Memory limit (e.g., "512Mi").
* `privileged` (boolean, optional): Whether to run the container in privileged mode.
* `storage_mounts` (list of objects, optional): Volumes to mount.

## Example

```hcl
step "build" "RunContainer" {
  spec = {
    image   = "golang:1.21-alpine"
    command = ["go"]
    args    = ["build", "-o", "bin/app", "./cmd/app"]
    env = [
      { name = "GOOS", value = "linux" },
      { name = "GOARCH", value = "amd64" }
    ]
    cpu    = "1000m"
    memory = "1Gi"
  }
}
```
