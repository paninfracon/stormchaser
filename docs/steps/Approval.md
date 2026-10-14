# Approval Step

The `Approval` step pauses workflow execution until manual approval is provided. This is commonly used in deployment pipelines for gated releases.

## DSL Specification

The `spec` block for `Approval` supports the following fields:

* `approvers` (list of strings, optional): List of users (by email or subject) or groups (values of the token's `groups` claim) allowed to approve or reject the step. Admins may too. Enforced by the shipped OPA policy, so only when the API has OPA configured; an emailed link counts as coming from the people it was sent to. Empty or absent, anyone who may see the run may decide.
* `inputs` (list of objects, optional): Forms/Inputs required from the approver before approval is granted.
* `notify` (object, optional): Email notification configuration (matches `EmailSend` spec).
* `timeout` (string, optional): Duration to wait for approval before timing out (e.g., "24h", "60m").

## Example

```hcl
step "manual_approval" "Approval" {
  spec = {
    approvers = ["admin_group", "user@example.com"]
    timeout   = "48h"
    notify = {
      to      = ["admin_group@example.com"]
      subject = "Deployment Approval Required"
      body    = "Please approve the production deployment."
    }
  }
}
```
