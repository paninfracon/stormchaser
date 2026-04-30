# TerraformPlan Step

The `TerraformPlan` step is a native intrinsic step that executes a `terraform plan` command within the official Terraform container. Under the hood, it seamlessly runs `terraform init` followed by `terraform plan` and stores the resulting plan binary into the workspace so that a subsequent `TerraformApply` step can execute it.

Because Stormchaser utilizes a shared Stormchaser File System (SFS) across steps within the same execution path, downloaded modules, state locks, and the `.tfplan` binary naturally persist.

**Automatic Plan Generation:** During execution, this step automatically runs `terraform show -no-color tfplan > plan.txt`. This human-readable text file is generated in the workspace and can be exposed as a step artifact.

**Plugin Caching:** This step automatically sets the `TF_PLUGIN_CACHE_DIR` environment variable to `/tmp/.terraform_plugin_cache`. Mounting a shared persistent volume to this path can significantly speed up the `terraform init` phase across multiple runs.

## DSL Specification

The `spec` block for `TerraformPlan` supports the following fields:

* `workspace_dir` (string, optional): The directory containing the Terraform configuration files. Defaults to `.` (the root).
* `backend_bucket` (string, optional): S3 bucket name if using an S3 remote backend.
* `backend_key` (string, optional): State file key path within the S3 bucket.
* `region` (string, optional): AWS Region for backend configuration and AWS provider. It also injects `AWS_REGION` into the environment.
* `aws_assume_role_arn` (string, optional): The ARN of an IAM role to assume before running Terraform. When provided (and if the engine is built with the `aws-sdk-sts` feature), Stormchaser will natively assume this role and inject short-lived credentials (`AWS_ACCESS_KEY_ID`, etc.) into the container.
* `aws_role_session_name` (string, optional): An optional session name to use when assuming the role. Defaults to `stormchaser-tf-<run_id>`.
* `out_file` (string, optional): Name of the plan file to generate. Defaults to `tfplan`.

## Example

```hcl
step "plan" "TerraformPlan" {
  spec {
    workspace_dir  = "infrastructure/"
    backend_bucket = "my-company-tf-state"
    backend_key    = "app/production.tfstate"
    region         = "us-east-1"
    out_file       = "tfplan"
  }
}
```
