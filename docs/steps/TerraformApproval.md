# TerraformApproval Step

The `TerraformApproval` step is a specialized intrinsic step designed to work seamlessly with `TerraformPlan`. It is a syntactic sugar over the standard Human-In-The-Loop (HITL) `Approval` step, pre-configured to present a clean summary of a Terraform plan to human approvers.

When combined with `TerraformPlan`, the engine automatically extracts the "plan summary" string (e.g. "Plan: 5 to add, 2 to change, 0 to destroy.") from the plan output and maps it to a workflow output. This step allows you to present that summary directly within the user's approval prompt via the `plan_summary` attribute.

## DSL Specification

The `spec` block for `TerraformApproval` supports the following fields:

* `approvers` (list of strings, optional): List of users or groups allowed to approve the step.
* `plan_summary` (string, optional): A text summary of the Terraform plan to display to approvers. Typically, this evaluates an expression from a `TerraformPlan` step.
* `timeout` (string, optional): Duration to wait for approval before timing out (e.g., "24h", "60m").
* `notify` (object, optional): Email notification configuration.

## Example

```hcl
step "plan" "TerraformPlan" {
  spec {
    workspace_dir  = "."
    out_file       = "tfplan"
  }
}

step "approve" "TerraformApproval" {
  spec {
    approvers    = ["admin_team"]
    plan_summary = "${steps.plan.outputs.plan_summary}"
    timeout      = "48h"
  }
}

step "apply" "TerraformApply" {
  spec {
    out_file = "tfplan"
  }
}
```

Under the hood, `TerraformApproval` automatically mutates into an `Approval` step containing a single required input named `approval_decision` with the options "Approve" and "Reject", accompanied by the provided `plan_summary` as context.
