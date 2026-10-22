# TerraformApply Step

The `TerraformApply` step runs `terraform init` and `terraform apply <out_file>` on a plan file that a [TerraformPlan](TerraformPlan.md) step saved. Like `TerraformPlan`, the engine turns it into a `RunContainer` step in the `hashicorp/terraform` image, and it needs the same storage mounted so that it can find the configuration and the plan.

After a successful apply, the step runs `terraform output -json` and stores the result in the output `terraform`. A later step reads an output variable as `${steps.<step_name>.outputs.terraform.<variable>.value}`.

It sets `TF_PLUGIN_CACHE_DIR=/tmp/.terraform_plugin_cache`, as `TerraformPlan` does.

## DSL Specification

The `spec` block for `TerraformApply` takes the same fields as `TerraformPlan` (`workspace_dir`, `backend_bucket`, `backend_key`, `region`, `aws_assume_role_arn`, `aws_role_session_name`, `out_file`, `storage_mounts`, `cpu`, `memory`), with `out_file` naming the plan to apply, and one more:

* `auto_approve` (boolean, optional): Pass `-auto-approve` to `terraform apply`. Defaults to `true`.

## Example

```hcl
step "apply" "TerraformApply" {
  spec {
    workspace_dir  = "/workspace/infrastructure"
    backend_bucket = "my-company-tf-state"
    backend_key    = "app/production.tfstate"
    region         = "us-east-1"
    storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
  }
  next = ["notify"]
}

step "notify" "WebhookInvoke" {
  spec {
    url  = "https://example.com/hooks/infra"
    // Reads a Terraform output variable named 'vpc_id'
    body = "{\"vpc_id\": \"${steps.apply.outputs.terraform.vpc_id.value}\"}"
  }
}
```
