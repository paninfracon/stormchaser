# LambdaInvoke Step

The `LambdaInvoke` step synchronously or asynchronously invokes an AWS Lambda function.

**Note:** The correct AWS IAM role and credentials must be in effect in the environment where the engine or runner is executing to successfully invoke the Lambda function.

## DSL Specification

The `spec` block for `LambdaInvoke` supports the following fields:

* `function_name` (string, required): The name or ARN of the Lambda function to invoke.
* `payload` (object, optional): JSON payload to pass to the function.
* `invocation_type` (string, optional): Type of invocation. Can be "RequestResponse", "Event", or "DryRun". Defaults to "RequestResponse".
* `qualifier` (string, optional): Lambda version or alias.
* `region` (string, optional): AWS region to execute the function in.
* `assume_role_arn` (string, optional): IAM Role ARN to assume before invoking the function. The operator must list the role in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`. Without a role the function is invoked with the engine's own credentials and must be listed in `STORMCHASER_AWS_ALLOWED_LAMBDA_FUNCTIONS`, as written in `function_name`.
* `role_session_name` (string, optional): Session name to use when assuming the role. Defaults to `stormchaser-run-<run_id>`.

## Example

```hcl
step "cleanup_resources" "LambdaInvoke" {
  spec = {
    function_name     = "ResourceCleanupWorker"
    invocation_type   = "Event"
    payload = {
      environment = "staging"
      action      = "teardown"
    }
    region          = "us-east-1"
    assume_role_arn = "arn:aws:iam::123456789012:role/cross-account-lambda-invoker"
  }
}
```
