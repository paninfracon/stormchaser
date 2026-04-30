# TerraformPlan Step

The `TerraformPlan` step is a native intrinsic step that executes a `terraform plan` command within the official Terraform container. Under the hood, it seamlessly runs `terraform init` followed by `terraform plan` and stores the resulting plan binary into the workspace so that a subsequent `TerraformApply` step can execute it.

Because Stormchaser utilizes a shared Stormchaser File System (SFS) across steps within the same execution path, downloaded modules, state locks, and the `.tfplan` binary naturally persist.

## DSL Specification

The `spec` block for `TerraformPlan` supports the following fields:

* `workspace_dir` (string, optional): The directory containing the Terraform configuration files. Defaults to `.` (the root).
* `backend_bucket` (string, optional): S3 bucket name if using an S3 remote backend.
* `backend_key` (string, optional): State file key path within the S3 bucket.
* `region` (string, optional): AWS Region for backend configuration and AWS provider. It also injects `AWS_REGION` into the environment.
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
