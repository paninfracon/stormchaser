# EmailSend Step

The `EmailSend` (or `Email`) step sends an email notification via an SMTP server.

## DSL Specification

The `spec` block for `EmailSend` supports the following fields:

* `from` (string, required): The sender's email address.
* `to` (list of strings, required): List of recipient email addresses.
* `cc` (list of strings, optional): Carbon copy recipients.
* `bcc` (list of strings, optional): Blind carbon copy recipients.
* `subject` (string, required): The email subject.
* `body` (string, required): The email body. Supports MiniJinja template rendering.
* `html` (boolean, optional): Whether the body is HTML formatted.
* `backend` (string, optional): The email delivery backend to use: `smtp` (default) or `ses`.
* `smtp_server` (string, optional): SMTP server address. Defaults to `SMTP_SERVER` env var.
* `smtp_port` (integer, optional): SMTP server port. Defaults to `SMTP_PORT` env var.
* `smtp_username` (string, optional): SMTP authentication username.
* `smtp_password` (string, optional): SMTP authentication password.
* `smtp_use_tls` (boolean, optional): Whether to use TLS for SMTP. Defaults to `SMTP_USE_TLS` env var.
* `smtp_use_mtls` (boolean, optional): Whether to use mutual TLS (mTLS) for SMTP. Defaults to `SMTP_USE_MTLS` env var.
* `ses_region` (string, optional): AWS region for SES.
* `ses_role_arn` (string, optional): IAM Role ARN to assume for sending via SES.
* `ses_configuration_set_name` (string, optional): SES configuration set name.

## Example

### SMTP with TLS

```hcl
step "notify_smtp" "EmailSend" {
  spec = {
    from         = "no-reply@paninfracon.net"
    to           = ["ops@paninfracon.net"]
    subject      = "Workflow Alert"
    body         = "Critical failure in run {{ run.id }}"
    smtp_use_tls = true
  }
}
```

### AWS SES

```hcl
step "notify_ses" "EmailSend" {
  spec = {
    backend      = "ses"
    from         = "ci@paninfracon.net"
    to           = ["devs@paninfracon.net"]
    subject      = "Build Succeeded"
    body         = "The build for {{ run.id }} is complete."
    ses_region   = "us-east-1"
  }
}
```
