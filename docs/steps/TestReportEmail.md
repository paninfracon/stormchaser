# TestReportEmail Step

The `TestReportEmail` step sends an HTML email with the test results of the current run: a summary of each report and a table of its failed and errored cases. The engine runs it itself; no runner is involved. A custom template can replace the default one.

It reads the reports that earlier steps declared in a `reports` block and that were parsed as `junit` (see [Test reports](../workflow_dsl.md#455-test-reports)), so it belongs after those steps. It needs an engine built with the `email` feature (on by default), and SES needs `aws-ses` too.

## DSL Specification

The `spec` block for `TestReportEmail` supports the following fields:

* `from` (string, required): The sender's email address.
* `to` (list of strings, required): The recipient email addresses.
* `subject` (string, required): The email subject line.
* `report_name` (string, optional): The name of a specific test report to include. If omitted, all reports from the run are included.
* `template` (string, optional): A MiniJinja template that replaces the default HTML report. The email is always sent as HTML.
* `backend` (string, optional): The email delivery backend to use: `smtp` (default) or `ses`.
* `smtp_server` (string, optional): SMTP server address. Defaults to `SMTP_SERVER` env var.
* `smtp_port` (integer, optional): SMTP server port. Defaults to `SMTP_PORT` env var.
* `smtp_username` (string, optional): SMTP authentication username. Defaults to `SMTP_USERNAME`, but only when the step uses the engine's own `SMTP_SERVER` and `SMTP_PORT`.
* `smtp_password` (string, optional): SMTP authentication password. Defaults to `SMTP_PASSWORD` under the same condition; a step that names another server must supply its own credentials.
* `smtp_use_tls` (boolean, optional): Require TLS. Defaults to the engine's `SMTP_USE_TLS`.
* `smtp_use_mtls` (boolean, optional): Not supported; setting it to `true` fails the step.
* `ses_region` (string, optional): AWS region for SES.
* `ses_role_arn` (string, optional): IAM role to assume for sending through SES. The operator must list the role in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`.
* `ses_configuration_set_name` (string, optional): SES configuration set name.

### Template Context

If you provide a custom `template`, it will have access to the following context:

* `reports`: A list of objects, each containing:
  * `summary`: The report's summary: `report_name`, `total_tests`, `passed`, `failed`, `errors`, `skipped` and `duration_ms`.
  * `cases`: Every test case of the report, each with `test_suite`, `test_case`, `status` (`passed`, `failed`, `error` or `skipped`), `duration_ms` and `message`.
* `inputs`: The workflow inputs.
* `steps`: Outputs of steps that have finished, as `steps.<name>.outputs.<key>`.
* `run`: Metadata about the current run (e.g., `run.id`).

## Example

```hcl
step "notify_results" "TestReportEmail" {
  spec = {
    from    = "ci@example.com"
    to      = ["dev-team@example.com"]
    subject = "Test results for run ${run.id}"
  }
}
```

### With Custom Template

```hcl
step "notify_custom" "TestReportEmail" {
  spec = {
    from    = "ci@example.com"
    to      = ["dev-team@example.com"]
    subject = "Custom test report"
    template = <<EOT
    <h1>Quick Summary</h1>
    {% for report in reports %}
      <p>{{ report.summary.report_name }}: {{ report.summary.passed }}/{{ report.summary.total_tests }} passed</p>
    {% endfor %}
EOT
  }
}
```
