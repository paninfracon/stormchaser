# Future Terraform Enhancements

This document outlines proposed enhancements to the native `TerraformPlan`, `TerraformApply`, and `TerraformApproval` intrinsic steps in Stormchaser.

## 1. Deep Terraform OPA Context (Resource-Level ABAC) (Implemented)

**Goal:** Extract the full JSON representation of the Terraform plan and inject it into the `ApprovalOpaContext`.
**Rationale:** Allows security and compliance teams to write granular Rego policies. For example, enforcing separation of duties if a specific sensitive resource (like `aws_iam_role`) is modified, or automatically denying plans that open security groups to `0.0.0.0/0`.
**Implementation:** `TerraformPlan` executes `terraform show -json tfplan` and extracts it natively into a `plan_json` output variable. During the HITL approval phase, the API resolves all run outputs and provides them inside the `ApprovalOpaContext` as `run_outputs`, allowing Rego policies to deeply inspect `input.run_outputs.terraform_plan.outputs.plan_json.resource_changes`.

## 2. Native OIDC Cloud Authentication (Implemented)

**Goal:** Eliminate long-lived static credentials for cloud providers.
**Rationale:** The current approach requires passing `AWS_ACCESS_KEY_ID` or `GOOGLE_CREDENTIALS` via environment variables. Since Stormchaser runs on modern orchestrators, it should natively generate short-lived OIDC tokens.
**Implementation:** You can now provide `aws_assume_role_arn` and `aws_role_session_name` in the spec of `TerraformPlan` and `TerraformApply`. If the Stormchaser Engine is compiled with the `aws-sdk-sts` feature, it will natively invoke `sts:AssumeRole` before dispatching the container and inject short-lived AWS credentials securely into the container environment.

## 3. Automatic Plan Artifact Generation (Implemented)

**Goal:** Automatically expose the human-readable plan text as a downloadable artifact.
**Rationale:** While the `TerraformApproval` UI shows a brief summary (e.g., "3 to add, 1 to destroy"), a human approver needs to read the full line-by-line diff before making an informed decision.
**Implementation:** `TerraformPlan` automatically executes `terraform show -no-color tfplan > plan.txt` during its run phase, exposing the readable plan file to the workspace.

## 4. Plugin Caching (`.terraform/providers`) (Implemented)

**Goal:** Speed up execution by caching provider binaries.
**Rationale:** `terraform init` downloads provider plugins (like the AWS provider, which can be hundreds of megabytes) every time it runs. In a workflow with multiple sub-modules, this is highly inefficient.
**Implementation:** `TerraformPlan` and `TerraformApply` automatically inject `TF_PLUGIN_CACHE_DIR=/tmp/.terraform_plugin_cache` and ensure the directory exists before `terraform init`. Users simply need to configure a persistent storage volume mount targeting this path to enable caching.
