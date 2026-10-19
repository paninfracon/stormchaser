# RunK8sJob Step

The `RunK8sJob` step creates and watches a Kubernetes Job, with more of the Job and pod spec open to the workflow than `RunContainer` has. Only the Kubernetes runner takes it.

## DSL Specification

The `spec` block for `RunK8sJob` maps to `K8sJobSpec` and supports the following fields:

* `image` (string, required): The image to run.
* `command` (list of strings, optional): Replaces the image's entrypoint.
* `args` (list of strings, optional): Arguments to the command.
* `env` (list of objects, optional): Environment variables, each `{ name = "...", value = "..." }`.
* `resources` (object, optional): `requests` and `limits`, each with `cpu` and `memory`. The container's limit is the value in `limits`, else the one in `requests`, else the runner's default (1 CPU, 2 GiB); a request is set to the limit if it is missing or larger. A step that asks for more than the runner's maximum (8 CPUs, 16 GiB by default) is refused.
* `secret_mounts` (list of objects, optional): Kubernetes Secrets to mount, each `{ name = "...", mount_path = "..." }`. Each name must be listed by the operator in `STORMCHASER_ALLOWED_SECRET_MOUNTS` on the runner.
* `config_map_mounts` (list of objects, optional): ConfigMaps to mount, each `{ name = "...", mount_path = "..." }`. Each name must be listed by the operator in `STORMCHASER_ALLOWED_CONFIG_MAP_MOUNTS` on the runner.
* `storage_mounts` (list of objects, optional): Storage to mount. `mount_path` must be absolute and must not be a system location; see [Mounting Storage in Steps](../workflow_dsl.md#43-mounting-storage-in-steps).
* `service_account_name` (string, optional): The pod's ServiceAccount. It must be listed by the operator in `STORMCHASER_ALLOWED_SERVICE_ACCOUNTS` on the runner.
* `registry_connection` (string, optional): As for [RunContainer](RunContainer.md).
* `connections` (list of strings, optional): As for [RunContainer](RunContainer.md).
* `privileged` (boolean, optional): Run the container privileged. Refused unless the runner has `STORMCHASER_ALLOW_PRIVILEGED=true`.
* `active_deadline_seconds` (integer, optional): The Job's deadline. The step's `timeout` (24 hours by default) applies too, and the shorter of the two is used.
* `backoff_limit` (integer, optional): How many times Kubernetes retries a failed pod within one attempt of the step. Defaults to 0. Use the step's `retry` block to run the whole step again instead.
* `completions` (integer, optional): Pods that must succeed. Defaults to 1.
* `parallelism` (integer, optional): Pods that may run at once. Defaults to 1. `completions` and `parallelism` may be at most the runner's maximum (10 by default).
* `ttl_seconds_after_finished` (integer, optional): How long Kubernetes keeps the finished Job. Defaults to 3600.
* `restart_policy` (string, optional): The pod's restart policy, `OnFailure` or `Never`.
* `node_selector` (map of strings, optional): The pod's node selector.
* `labels` (map of strings, optional): Extra labels on the Job.
* `annotations` (map of strings, optional): Extra annotations on the Job.
* `minimum_version` (string, optional): The lowest Kubernetes version the step accepts. On an older cluster the step fails before the Job is created.

## Example

```hcl
step "db_migration" "RunK8sJob" {
  spec = {
    image         = "my-app-migrations:latest"
    command       = ["/migrate"]
    args          = ["up"]
    backoff_limit = 3
    resources = {
      requests = { cpu = "500m", memory = "256Mi" }
      limits   = { cpu = "1000m", memory = "512Mi" }
    }
  }
}
```
