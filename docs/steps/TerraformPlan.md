# TerraformPlan Step

The `TerraformPlan` step runs `terraform init` and `terraform plan -out=<out_file>`, and keeps the plan file for a later [TerraformApply](TerraformApply.md) step. The engine turns it into a `RunContainer` step that runs in the `hashicorp/terraform` image (1.16.6, pinned by digest; the operator can replace it with `STORMCHASER_TERRAFORM_IMAGE`).

The configuration has to be in the container, and the plan file has to outlive it, so mount storage that holds the configuration (put there by a [GitCheckout](GitCheckout.md) step, for example) and point `workspace_dir` into it. The plan and apply steps must mount the same storage.

After planning, the step also:

* writes `terraform show -no-color` of the plan to `plan.txt` in `workspace_dir`;
* sets the output `plan_summary` to the plan's summary line (`Plan: 5 to add, 2 to change, 0 to destroy.` or `No changes. ...`);
* sets the output `plan_json` to `terraform show -json` of the plan.

It sets `TF_PLUGIN_CACHE_DIR=/tmp/.terraform_plugin_cache`; mounting storage there keeps provider downloads between steps.

## DSL Specification

The `spec` block for `TerraformPlan` supports the following fields:

* `workspace_dir` (string, optional): The directory with the Terraform configuration. Defaults to `.`, the container's working directory.
* `backend_bucket` (string, optional): Passed to `terraform init` as `-backend-config="bucket=..."`.
* `backend_key` (string, optional): Passed as `-backend-config="key=..."`.
* `region` (string, optional): Passed as `-backend-config="region=..."`, and set as `AWS_REGION`.
* `aws_assume_role_arn` (string, optional): An IAM role the engine assumes before the step starts. Its short-lived credentials are given to the container as `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`. The operator must list the role in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`. Needs the engine's `aws-sdk-sts` feature (on by default); without it the field is ignored.
* `aws_role_session_name` (string, optional): Session name for the assumed role. Defaults to `stormchaser-tf-<run_id>`.
* `out_file` (string, optional): The plan file's name, relative to `workspace_dir`. Defaults to `tfplan`. It must not be empty or start with `-`.
* `storage_mounts` (list of objects, optional): Storage to mount, as for [RunContainer](RunContainer.md).
* `cpu`, `memory` (strings, optional): As for [RunContainer](RunContainer.md).

## Example

```hcl
step "plan" "TerraformPlan" {
  spec {
    workspace_dir  = "/workspace/infrastructure"
    backend_bucket = "my-company-tf-state"
    backend_key    = "app/production.tfstate"
    region         = "us-east-1"
    storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
  }
  next = ["approve"]
}
```
