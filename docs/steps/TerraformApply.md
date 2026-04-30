# TerraformApply Step

The `TerraformApply` step natively executes a previously generated `.tfplan` using HashiCorp's official container image.

Like `TerraformPlan`, the execution runs on top of the shared file system context, meaning module dependencies and lock states do not need to be manually restored.

**Outputs Capture:** A key feature of `TerraformApply` is that it natively extracts JSON output variables after a successful apply. The values produced by `terraform output -json` are parsed into a native `terraform` object available to subsequent workflow steps via the `steps.<step_name>.outputs.terraform` expression.

**Plugin Caching:** Similar to `TerraformPlan`, this step automatically sets the `TF_PLUGIN_CACHE_DIR` environment variable to `/tmp/.terraform_plugin_cache`. Mounting a shared persistent volume to this path can significantly speed up the `terraform init` phase.

## DSL Specification

The `spec` block for `TerraformApply` shares similar configuration with `TerraformPlan`:

* `workspace_dir` (string, optional): The directory containing the Terraform configuration files. Defaults to `.` (the root).
* `backend_bucket` (string, optional): S3 bucket name if using an S3 remote backend.
* `backend_key` (string, optional): State file key path within the S3 bucket.
* `region` (string, optional): AWS Region for backend configuration and AWS provider. Injects `AWS_REGION`.
* `out_file` (string, optional): Name of the plan file to execute. Defaults to `tfplan`.
* `auto_approve` (boolean, optional): Whether to run with `-auto-approve`. Defaults to `true`.

## Example

```hcl
step "apply" "TerraformApply" {
  spec {
    workspace_dir  = "infrastructure/"
    backend_bucket = "my-company-tf-state"
    backend_key    = "app/production.tfstate"
    region         = "us-east-1"
    out_file       = "tfplan"
  }
}

step "notify" "WebhookInvoke" {
  spec {
    url = "https://example.com"
    body = {
      // Accessing a terraform output variable named 'vpc_id'
      vpc_id = "${steps.apply.outputs.terraform.vpc_id.value}"
    }
  }
}
```
