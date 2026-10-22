# Terraform Steps

Stormchaser has three built-in Terraform step types: `TerraformPlan`,
`TerraformApply` and `TerraformApproval`. Their fields are listed in
[TerraformPlan](steps/TerraformPlan.md), [TerraformApply](steps/TerraformApply.md)
and [TerraformApproval](steps/TerraformApproval.md). This page records how they
work and which of the planned enhancements are in.

## How they run

When the engine dispatches a `TerraformPlan` or `TerraformApply` step, it
rewrites it into a `RunContainer` step:

* The image is `hashicorp/terraform:1.16.6`, pinned by digest. The engine's
  `STORMCHASER_TERRAFORM_IMAGE` replaces it.
* The script runs `terraform init` in `workspace_dir` (default `.`), with
  `backend_bucket`, `backend_key` and `region` passed as `-backend-config` when
  set, then `terraform plan -out=<out_file>` or `terraform apply <out_file>`
  (`out_file` defaults to `tfplan`; `auto_approve`, default true, adds
  `-auto-approve`).
* Spec values reach the script as environment variables, never as part of its
  text. `region` also sets `AWS_REGION`.
* `storage_mounts`, `cpu` and `memory` pass through to the container.

`TerraformApply` applies the plan file, so the plan and apply steps must share
the workspace through a storage mount.

`TerraformApproval` becomes an `Approval` step with one input,
`approval_decision` (`Approve` or `Reject`). Its description is `plan_summary`;
if the summary says resources will be destroyed, the description and any
`notify` email get a warning. `approvers`, `notify` and `timeout` pass through.

Outputs are read back from the step's logs, so the engine needs a log backend
(`LOKI_URL`):

* `TerraformPlan`: `plan_summary` (the `Plan:` or `No changes.` line) and
  `plan_json` (`terraform show -json`).
* `TerraformApply`: `terraform` (`terraform output -json`).

None of these outputs is marked sensitive, and they are printed to the step's
logs. Keep secrets out of plan output and Terraform outputs, or mark them
`sensitive` in Terraform.

## 1. Deep Terraform OPA Context (Resource-Level ABAC) (Implemented)

**Goal:** Give approval policies the full JSON plan.
**Rationale:** Lets security teams write Rego that inspects individual resource
changes, for example requiring a second approver when an `aws_iam_role` changes,
or denying plans that open security groups to `0.0.0.0/0`.
**Implementation:** `TerraformPlan` runs `terraform show -json` and stores the
result as the `plan_json` output. When someone approves a step, the API puts all
the run's outputs into the `ApprovalOpaContext` as `run_outputs`, so a policy
can read `input.run_outputs.<plan step name>.outputs.plan_json.resource_changes`.

## 2. Native OIDC Cloud Authentication (Implemented, AWS only)

**Goal:** No long-lived cloud credentials in workflows.
**Implementation:** Set `aws_assume_role_arn` (and optionally
`aws_role_session_name`, default `stormchaser-tf-<run id>`) in the spec of
`TerraformPlan` or `TerraformApply`. The engine calls `sts:AssumeRole` with its
own AWS credentials before dispatching the step and passes the short-lived
credentials to the container as environment variables. This needs the engine's
`aws-sdk-sts` Cargo feature, which is on by default. The role must be listed in
the engine's `STORMCHASER_AWS_ALLOWED_ROLE_ARNS` (Helm: `global.aws.allowedRoleArns`);
the list is empty by default, and a trailing `*` matches a prefix.

There is no equivalent for other clouds.

## 3. Plan Text in the Workspace (Implemented)

**Goal:** Let approvers read the full plan, not only the summary line.
**Implementation:** `TerraformPlan` runs `terraform show -no-color` and writes
the result to `plan.txt` in `workspace_dir`. It is a file in the workspace, not a
published artifact. A later step that mounts the same storage can read it.

## 4. Plugin Caching (Implemented)

**Goal:** Avoid downloading provider binaries on every `terraform init`.
**Implementation:** `TerraformPlan` and `TerraformApply` set
`TF_PLUGIN_CACHE_DIR=/tmp/.terraform_plugin_cache` and create the directory
before `terraform init`. Mount persistent storage at that path to keep the cache
between steps.
