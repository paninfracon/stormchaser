# RunK8sJob Step

The `RunK8sJob` step creates and monitors a Kubernetes Job. This is useful for executing workloads directly on a Kubernetes cluster.

## DSL Specification

The `spec` block for `RunK8sJob` maps to the `K8sJobSpec` and supports the following fields:

* `image` (string, required): The Docker image to run.
* `command` (list of strings, optional): Overrides the default entrypoint.
* `args` (list of strings, optional): Arguments to pass to the command.
* `env` (list of objects, optional): Environment variables.
* `resources` (object, optional): Kubernetes resource requests and limits.
* `secret_mounts` (list of objects, optional): Kubernetes secrets to mount. Each name must be listed by the operator in `STORMCHASER_ALLOWED_SECRET_MOUNTS` on the runner.
* `config_map_mounts` (list of objects, optional): Kubernetes config maps to mount. Each name must be listed by the operator in `STORMCHASER_ALLOWED_CONFIG_MAP_MOUNTS` on the runner.
* `storage_mounts` (list of objects, optional): Volumes to mount. `mount_path` must be absolute and must not be a system location; see [Mounting Storage in Steps](../workflow_dsl.md).
* `active_deadline_seconds` (integer, optional): Job active deadline.
* `backoff_limit` (integer, optional): Number of retries before marking as failed.
* `completions` (integer, optional): Number of successfully finished pods required.
* `parallelism` (integer, optional): Maximum number of pods that can run in parallel.
* `ttl_seconds_after_finished` (integer, optional): TTL for cleaning up the job.

## Example

```hcl
step "db_migration" "RunK8sJob" {
  spec = {
    image   = "my-app-migrations:latest"
    command = ["/migrate"]
    args    = ["up"]
    backoff_limit = 3
    completions   = 1
    parallelism   = 1
    resources = {
      requests = { cpu = "500m", memory = "256Mi" }
      limits   = { cpu = "1000m", memory = "512Mi" }
    }
  }
}
```
