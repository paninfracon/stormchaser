# Observability in Stormchaser

Stormchaser emits metrics and traces over OpenTelemetry (OTLP) and writes logs
to stdout. It reads step logs back from Grafana Loki. This page lists what is
emitted and what each component needs, so you can point it at your own
platform.

## Bundled stack (quickstart only)

docker-compose and the umbrella Helm chart bundle the components below so that
the quickstart works out of the box. They are for evaluation, not production:
Loki there has no authentication, Grafana lets anonymous visitors view
dashboards, and none of it is sized, secured or backed up. In production, send
logs, metrics and traces to your existing platform; see the
[production prerequisites](environments.md#cluster-prerequisites).

* **Prometheus**: stores metrics (received by remote write from Alloy).
* **Grafana Loki**: stores logs from the control plane and steps.
* **Grafana Tempo**: stores traces.
* **Grafana Alloy**: receives OTLP from the API and engine, collects pod logs,
  and forwards them to the backends above.
* **Grafana**: dashboards and exploration, with the three backends configured as
  datasources.

## What you need to provide

| Component | Needs | Setting |
| :--- | :--- | :--- |
| API, engine | An OTLP **gRPC** endpoint for traces and metrics | `OTEL_EXPORTER_OTLP_ENDPOINT` (Helm: `global.otel.endpoint`) |
| API, engine | A Loki endpoint to read step logs from | `LOKI_URL` (Helm: `global.loki.url`) |
| Docker runner | A Loki endpoint to push step logs to, and the Loki Docker logging driver installed on the host as `loki` | `LOKI_URL` |
| Kubernetes runner | A log collector that ships step pod logs to Loki with a `job_name` label (see [Logs](#logs)) | Your collector's configuration |
| All services | Collection of stdout logs | Your collector's configuration |

If `OTEL_EXPORTER_OTLP_ENDPOINT` is not set, the API and engine export no
traces or metrics. `OTEL_SERVICE_NAME` sets the `service.name` resource
attribute (default `stormchaser-api` or `stormchaser-engine`). The exporter
speaks OTLP over gRPC only; an HTTP-only (port 4318) endpoint will not work.

If `LOKI_URL` is not set, the API cannot serve step logs to the CLI, TUI and
web UI, and the engine cannot read step outputs that are extracted from logs
(output extractions with `source = "logs"` or `"stdout"`, including the
Terraform steps' outputs). Loki is the only log backend that can be configured.

The runners, the agent and the query service export no traces or metrics.

## Tracing

The API and engine use `tracing` with an OpenTelemetry layer, so their spans are
exported when an endpoint is set.

* **HTTP requests:** the API makes a server span per request (`HTTP <method>`)
  with the method, path and HTTP version. Approval-link tokens are redacted from
  the path.
* **Run correlation:** API handlers that create runs, and engine handlers for
  run and step events, state transitions, dispatch and archiving, carry a
  `run_id` attribute. Step-level spans also carry `step_id`. Search Tempo for
  `run_id` to find a run's spans.
* **No cross-service traces:** trace context is not propagated over NATS, so
  the API's spans and the engine's spans for the same run are separate traces.
  Correlate them by `run_id`.
* **No database spans:** SQL queries are not traced.

## Metrics

The API and engine export these over OTLP every 60 seconds (the SDK default).
Names below are the OpenTelemetry names; a Prometheus backend usually rewrites
the dots to underscores and adds `_total` to counters (for example
`stormchaser_v1_runs_started_total`).

| Metric Name | Type | Emitted by | Attributes | Description |
| :--- | :--- | :--- | :--- | :--- |
| `stormchaser.v1.runs_enqueued` | Counter | API | `workflow_name`, `initiating_user` | Runs queued through `POST /api/v1/runs` |
| `stormchaser.v1.runs_started` | Counter | engine | `workflow_name`, `initiating_user` | Runs that began execution |
| `stormchaser.v1.runs_completed` | Counter | engine | `workflow_name`, `initiating_user` | Runs that finished successfully |
| `stormchaser.v1.runs_failed` | Counter | engine | `workflow_name`, `initiating_user`, `error` | Runs failed because a step failed |
| `stormchaser.v1.steps_started` | Counter | engine | `step_name`, `step_type`, `runner_id` | Steps a runner reported as running |
| `stormchaser.v1.steps_completed` | Counter | engine | `step_name`, `step_type`, `runner_id` | Steps that succeeded |
| `stormchaser.v1.steps_failed` | Counter | engine | `step_name`, `step_type`, `runner_id`, `error` | Steps that failed for good (not counting attempts that were retried) |
| `stormchaser.v1.steps_retried` | Counter | engine | `step_name`, `step_type` | Failed step attempts that the step's retry policy runs again |
| `stormchaser.v1.step_duration_seconds` | Histogram | engine | as for `steps_completed` / `steps_failed` | Time from a step starting to it completing or failing for good |

Notes:

* `runs_enqueued` counts only runs queued through `POST /api/v1/runs`. Direct
  runs, webhook rules and cron schedules do not add to it.
* `runs_failed` counts runs failed by a step. Runs aborted by their timeout, and
  runs the engine fails before they start, are not counted.
* The `error` attribute holds the failure message, and `initiating_user` the
  user's id, so these series can have high cardinality. Drop or aggregate those
  attributes in your collector if your backend charges by series.

## Logs

### Service logs

The API, engine, query service, runners and agent log plain text to stdout, at
the level set by `RUST_LOG` (Helm: `config.rustLog` on the runner charts). There
is no JSON log format. Collect them as you would any container's output.

### Step logs

Step logs go to Loki, and the API and engine read them back with a Loki
`query_range` (or `tail` for live logs) on the label `job_name`, matched
exactly against `storm-<step name>-<first 8 characters of the step id>` (step
name lowercased, other characters than `a-z`, `0-9`, `-` and `.` replaced by
`-`).

Known issue: both runners name a step's container or Job with the step's full
id (`storm-<step name>-<step id>`), so this query does not find their logs.
Steps with `shared` affinity on Docker run as `docker exec` in a shared
container; their output goes to the Docker runner's own log (prefixed
`[exec]`), not to Loki.

* **Docker runner:** with `LOKI_URL` set, each step container uses Docker's
  `loki` logging driver, with the labels `job_name` (the container name),
  `run_id` and `step_id`. The `stormchaser-runner-docker` chart does not set
  `LOKI_URL`.
* **Kubernetes runner:** the runner does not ship logs. A collector on the nodes
  must read the step pods' logs and give them a `job_name` label equal to the
  pod's `job-name` label (the Job name). The bundled Alloy configuration in the
  `stormchaser-telemetry` chart does this, keeps only pods labelled
  `managed-by=stormchaser`, and adds `namespace`, `pod`, `container`,
  `stormchaser_run_id` and `stormchaser_step_id`.

## Querying Data in Grafana (bundled stack)

The bundled Grafana has these datasources:

1. **Prometheus**: for metrics.
2. **Loki**: for logs.
3. **Tempo**: for traces.

### Example: Finding a Run's Traces

1. Open Grafana.
2. Go to **Explore** and select the **Tempo** datasource.
3. Use the **Search** tab and filter on the span attribute `run_id`.
4. Open a trace to see the engine's or the API's work on that run.
