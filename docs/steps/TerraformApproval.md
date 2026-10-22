# TerraformApproval Step

The `TerraformApproval` step is an [Approval](Approval.md) step made for reviewing a [TerraformPlan](TerraformPlan.md). When it is scheduled the engine turns it into an `Approval` step whose `inputs` is a JSON schema with one required field, `approval_decision`: a choice of `Approve` (the default) or `Reject`, described as `Terraform Plan Review: <plan_summary>`.

If `plan_summary` says something will be destroyed (`N to destroy` with N above zero), the description starts with a destructive-changes warning, and a `notify` email gets a warning banner at the top of its body and `[WARNING]` at the start of its subject.

Deciding works as for `Approval`. Note that `approval_decision` is only a value the approver sends: approving the step with `approval_decision=Reject` still completes it. To stop the run, reject the step.

## DSL Specification

The `spec` block for `TerraformApproval` supports the following fields:

* `approvers` (list of strings, optional): As for [Approval](Approval.md).
* `plan_summary` (string, optional): The plan summary to show, usually `${steps.<plan step>.outputs.plan_summary}`. Defaults to `Review Terraform Plan`.
* `notify` (object, optional): An email sent when the step is reached, as for [Approval](Approval.md).
* `timeout` (string, optional): Stored but not enforced, as for [Approval](Approval.md).

## Example

```hcl
step "plan" "TerraformPlan" {
  spec {
    workspace_dir  = "/workspace/infrastructure"
    storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
  }
  next = ["approve"]
}

step "approve" "TerraformApproval" {
  spec {
    approvers    = ["infra-admins"]
    plan_summary = "${steps.plan.outputs.plan_summary}"
  }
  next = ["apply"]
}

step "apply" "TerraformApply" {
  spec {
    workspace_dir  = "/workspace/infrastructure"
    storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
  }
}
```
