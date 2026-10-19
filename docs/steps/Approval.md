# Approval Step

The `Approval` step pauses its run until a person approves or rejects it. It is commonly used to gate a release.

The step starts in the `waiting_for_event` state and is never sent to a runner. Approving it completes the step; rejecting it fails the step with "Rejected by human", which fails the run unless the step sets `allow_failure`. An `Approval` step is never retried.

## DSL Specification

The `spec` block for `Approval` supports the following fields:

* `approvers` (list of strings, optional): Users (by email or subject) or groups (values of the token's `groups` claim) allowed to approve or reject the step. Admins may too. Enforced by the shipped OPA policy, so only when the API has OPA configured; an emailed link counts as coming from the people it was sent to. Empty or absent, anyone who may see the run may decide.
* `inputs` (any value, optional): A description of the values the approver is asked for, stored with the step for clients to show. The engine does not check the values an approver sends against it.
* `notify` (object, optional): An email sent when the step is reached. It takes the same fields as [EmailSend](EmailSend.md), so `from`, `to`, `subject` and `body` are required; a `notify` block that lacks one of them is ignored and no email is sent. The `body` is a MiniJinja template with `inputs`, `run`, `approve_link` and `reject_link` in its context. The links are empty unless the operator has set `STORMCHASER_APPROVAL_LINK_KEY`; a link opens a confirmation page, expires (72 hours by default), and records the decision against the addresses in `to`.
* `timeout` (string, optional): Stored with the step but not enforced. A step left waiting ends only when its run times out (the run's `quotas.timeout`, held to the operator's ceiling).

## Deciding

* `stormchaser runs pending` lists steps waiting for a decision.
* `stormchaser runs approve <run_id> <step_id> --input key=value ...` approves the step (`POST /api/v1/runs/{run_id}/steps/{step_id}/approve`). The JSON body's top-level fields become the step's outputs, so a later step can read `${steps.<name>.outputs.key}`.
* `stormchaser runs reject <run_id> <step_id>` rejects it (`POST /api/v1/runs/{run_id}/steps/{step_id}/reject`).
* `stormchaser runs approve-link <token>` acts on an emailed link.

## Example

```hcl
step "manual_approval" "Approval" {
  spec = {
    approvers = ["release-managers", "alice@example.com"]
    notify = {
      from    = "stormchaser@example.com"
      to      = ["release-managers@example.com"]
      subject = "Approval required for run ${run.id}"
      body    = "Approve: {{ approve_link }}\nReject: {{ reject_link }}"
    }
  }
  next = ["deploy"]
}
```
