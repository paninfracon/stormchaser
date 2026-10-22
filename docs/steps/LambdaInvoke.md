# LambdaInvoke Step

The `LambdaInvoke` step invokes an AWS Lambda function. The engine runs it itself, with the AWS credentials of the engine's environment or of a role it assumes; no runner is involved.

It needs an engine built with the `aws-lambda` feature (on by default). In a build without it the step fails, saying the feature is not enabled.

## DSL Specification

The `spec` block for `LambdaInvoke` supports the following fields:

* `function_name` (string, required): The name or ARN of the function.
* `payload` (any value, optional): The JSON payload to send. Without it the function is invoked with an empty payload.
* `invocation_type` (string, optional): `RequestResponse` (the default), `Event` or `DryRun`. Any other value is read as `RequestResponse`.
* `qualifier` (string, optional): A version or alias of the function.
* `region` (string, optional): The AWS region to call.
* `assume_role_arn` (string, optional): An IAM role to assume before invoking the function. The operator must list the role in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`. Without a role the function is invoked with the engine's own credentials and must be listed in `STORMCHASER_AWS_ALLOWED_LAMBDA_FUNCTIONS`, as written in `function_name`.
* `role_session_name` (string, optional): Session name for the assumed role. Defaults to `stormchaser-run-<run_id>`.

### Outputs

* `response`: The function's response, parsed as JSON, or as a string if it is not JSON. A status code outside 200-299 fails the step.

## Example

```hcl
step "cleanup_resources" "LambdaInvoke" {
  spec = {
    function_name   = "ResourceCleanupWorker"
    invocation_type = "Event"
    payload = {
      environment = "staging"
      action      = "teardown"
      run_id      = "${run.id}"
    }
    region          = "us-east-1"
    assume_role_arn = "arn:aws:iam::123456789012:role/cross-account-lambda-invoker"
  }
}
```
