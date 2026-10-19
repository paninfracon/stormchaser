# RunContainer Step

The `RunContainer` step runs a container image. It is the general-purpose step for scripts, builds and tests. Both runners take it: the Docker runner runs a container, and the Kubernetes runner runs a Job.

## DSL Specification

The `spec` block for `RunContainer` maps to `CommonContainerSpec` and supports the following fields:

* `image` (string, required): The image to run.
* `command` (list of strings, optional): Replaces the image's entrypoint.
* `args` (list of strings, optional): Arguments to the command.
* `env` (list of objects, optional): Environment variables, each `{ name = "...", value = "..." }`.
* `cpu` (string, optional): CPU for the step, in Kubernetes notation (`"500m"`, `"2"`). Defaults to the runner's default (1 CPU), and may be at most the runner's maximum (8 by default).
* `memory` (string, optional): Memory for the step (`"512Mi"`, `"4Gi"`). Defaults to the runner's default (2 GiB), and may be at most the runner's maximum (16 GiB by default).
* `privileged` (boolean, optional): Run the container privileged. Runners refuse privileged steps unless the operator starts them with `STORMCHASER_ALLOW_PRIVILEGED=true`.
* `storage_mounts` (list of objects, optional): Storage to mount, each `{ name = "...", mount_path = "...", read_only = false }`. `mount_path` must be absolute and must not be a system location; see [Mounting Storage in Steps](../workflow_dsl.md#43-mounting-storage-in-steps).
* `registry_connection` (string, optional): The name of an `oci` or `jfrog` connection whose credentials are used to pull the image. The image must be on that connection's registry; the step fails otherwise.
* `connections` (list of strings, optional): Connections whose settings are given to the container as environment variables: `STORMCHASER_CONN_<NAME>_URL`, `STORMCHASER_CONN_<NAME>_USERNAME` and `STORMCHASER_CONN_<NAME>_PASSWORD`, where `<NAME>` is the connection name in upper case with `-` replaced by `_`. Only the values the connection has are set. The run's user must be allowed to use each connection, and the password is masked in logs.

A step's run time is limited by its `timeout` (see [Step limits](../workflow_dsl.md#48-step-limits)). A non-zero exit code fails the step.

The `spec { ... }` block, the `spec = { ... }` attribute and attributes written directly in the step body all set the same spec. The examples on this page use `spec = { ... }`.

## Example

```hcl
step "build" "RunContainer" {
  spec = {
    image   = "golang:1.23-alpine"
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
