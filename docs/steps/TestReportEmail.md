# TestReportEmail Step

The `TestReportEmail` step sends a formatted HTML email containing test summaries and failed test cases from the current workflow run. It uses a rich default template but allows for complete user customization.

## DSL Specification

The `spec` block for `TestReportEmail` supports the following fields:

* `from` (string, required): The sender's email address.
* `to` (list of strings, required): The recipient email addresses.
* `subject` (string, required): The email subject line.
* `report_name` (string, optional): The name of a specific test report to include. If omitted, all reports from the run are included.
* `template` (string, optional): An optional MiniJinja template to override the default HTML report.
* `backend` (string, optional): The email delivery backend to use: `smtp` (default) or `ses`.
* `smtp_server` (string, optional): SMTP server address. Defaults to `SMTP_SERVER` env var.
* `smtp_port` (integer, optional): SMTP server port. Defaults to `SMTP_PORT` env var.
* `smtp_username` (string, optional): SMTP authentication username.
* `smtp_password` (string, optional): SMTP authentication password.
* `smtp_use_tls` (boolean, optional): Whether to use TLS for SMTP.
* `smtp_use_mtls` (boolean, optional): Whether to use mutual TLS (mTLS) for SMTP.
* `ses_region` (string, optional): AWS region for SES.
* `ses_role_arn` (string, optional): IAM Role ARN to assume for sending via SES.
* `ses_configuration_set_name` (string, optional): SES configuration set name.

### Template Context

If you provide a custom `template`, it will have access to the following context:

* `reports`: A list of objects, each containing:
  * `summary`: The `TestSummary` object (total, passed, failed, errors, skipped, duration).
  * `cases`: A list of `TestCase` objects for failed/errored tests.
* `inputs`: The workflow inputs.
* `steps`: Outputs from previous steps.
* `run`: Metadata about the current run (e.g., `run.id`).

## Example

```hcl
step "notify_results" "TestReportEmail" {
  spec = {
    from    = "ci@paninfracon.net"
    to      = ["dev-team@paninfracon.net"]
    subject = "Test Results for Run ${run.id}"
  }
}
```

### With Custom Template

```hcl
step "notify_custom" "TestReportEmail" {
  spec = {
    from    = "ci@paninfracon.net"
    to      = ["dev-team@paninfracon.net"]
    subject = "Custom Test Report"
    template = <<EOT
    <h1>Quick Summary</h1>
    {% for report in reports %}
      <p>{{ report.summary.report_name }}: {{ report.summary.passed }}/{{ report.summary.total_tests }} passed</p>
    {% endfor %}
EOT
  }
}
```
