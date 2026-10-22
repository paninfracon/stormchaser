# EmailSend Step

The `EmailSend` step sends an email, over SMTP or through Amazon SES. The engine runs it itself; no runner is involved.

It needs an engine built with the `email` feature (on by default), and SES needs `aws-ses` too. In a build without them the step fails, saying the feature is not enabled.

## DSL Specification

The `spec` block for `EmailSend` supports the following fields:

* `from` (string, required): The sender's email address.
* `to` (list of strings, required): Recipient email addresses.
* `cc` (list of strings, optional): Carbon copy recipients.
* `bcc` (list of strings, optional): Blind carbon copy recipients.
* `subject` (string, required): The email subject.
* `body` (string, required): The email body, a MiniJinja template with `inputs`, `steps` (`steps.<name>.outputs.<key>`) and `run` (`run.id`) in its context. See [`${...}` inside Jinja templates](../workflow_dsl.md#521--inside-jinja-templates).
* `html` (boolean, optional): Send the body as HTML. Defaults to `false`.
* `backend` (string, optional): `smtp` (the default) or `ses`.
* `smtp_server` (string, optional): SMTP server address. Defaults to the engine's `SMTP_SERVER` (`localhost` if unset).
* `smtp_port` (integer, optional): SMTP server port. Defaults to the engine's `SMTP_PORT` (`25` if unset).
* `smtp_username` (string, optional): SMTP username. Defaults to `SMTP_USERNAME`, but only when the step uses the engine's own `SMTP_SERVER` and `SMTP_PORT`.
* `smtp_password` (string, optional): SMTP password. Defaults to `SMTP_PASSWORD` under the same condition; a step that names another server must supply its own credentials, or the message is sent without authentication.
* `smtp_use_tls` (boolean, optional): Require TLS. Defaults to the engine's `SMTP_USE_TLS`. A step that uses the engine's password cannot turn off TLS that the engine has on.
* `smtp_use_mtls` (boolean, optional): Not supported: a step that sets it to `true`, or runs on an engine with `SMTP_USE_MTLS=true`, fails.
* `ses_region` (string, optional): AWS region for SES.
* `ses_role_arn` (string, optional): IAM role to assume for sending through SES. The operator must list the role in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`.
* `ses_configuration_set_name` (string, optional): SES configuration set name.

The step has no outputs. It fails if the message cannot be sent.

## Examples

### SMTP with TLS

```hcl
step "notify_smtp" "EmailSend" {
  spec = {
    from         = "no-reply@example.com"
    to           = ["ops@example.com"]
    subject      = "Workflow alert"
    body         = "Run {{ run.id }} needs attention."
    smtp_use_tls = true
  }
}
```

### AWS SES

```hcl
step "notify_ses" "EmailSend" {
  spec = {
    backend    = "ses"
    from       = "ci@example.com"
    to         = ["devs@example.com"]
    subject    = "Build succeeded"
    body       = "The build for {{ run.id }} is complete."
    ses_region = "us-east-1"
  }
}
```
