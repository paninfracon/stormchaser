# Observability in Stormchaser

Stormchaser provides deep visibility into its internal operations through a
integrated observability stack including metrics, logs, and distributed
tracing.

## Stack Components

* **Prometheus**: Stores time-series metrics.
* **Grafana Loki**: Stores and indexes logs from the control plane and runners.
* **Grafana Tempo**: Provides distributed tracing to visualize the lifecycle of
  workflow runs.
* **Grafana Alloy**: The unified telemetry agent that collects, processes, and
  forwards data to the above backends.
* **Grafana**: The visualization frontend for all observability data.

## Tracing

Stormchaser uses OpenTelemetry (OTEL) for distributed tracing. Every major
operation—from API requests to engine state transitions—is recorded as a span.

### Key Features

* **Run Correlation**: All spans related to a specific workflow execution are
  tagged with a `run_id`. You can search for this ID in Grafana Tempo to see
  the complete history of a run.
* **Step Context**: Spans related to specific steps include a `step_id` and
  `step_name`.
* **Database Spans**: All SQL queries performed by the API and Engine are
  automatically captured as child spans, allowing you to debug performance
  issues at the database layer.

## Metrics

The system exports various business and technical metrics:

| Metric Name | Type | Description |
| :--- | :--- | :--- |
| `stormchaser.runs_enqueued` | Counter | Workflows submitted via the API |
| `stormchaser.runs_started` | Counter | Workflows that have begun execution |
| `stormchaser.runs_completed` | Counter | Workflows finished successfully |
| `stormchaser.runs_failed` | Counter | Workflows that failed |
| `stormchaser.steps_started` | Counter | Individual steps started running |
| `stormchaser.steps_completed` | Counter | Steps that succeeded |
| `stormchaser.steps_failed` | Counter | Steps that failed |
| `stormchaser.step_duration_seconds` | Histogram | Time taken for steps |

## Logs

Logs are collected from:

1. **Control Plane**: API and Engine service logs.
2. **Workflows**: Real-time output from containers running workflow steps.

Workflow logs are automatically labeled with `run_id` and `step_id` in Loki,
enabling the API to serve them back to the CLI and TUI without manual
configuration.

## Querying Data in Grafana

When using the built-in Grafana instance (included in the umbrella chart or
`docker-compose`), the following datasources are pre-configured:

1. **Prometheus**: For metrics dashboards.
2. **Loki**: For log exploration.
3. **Tempo**: For trace visualization.

### Example: Visualizing a Workflow Run

1. Open Grafana.
2. Go to **Explore** and select the **Tempo** datasource.
3. Search by **Trace ID** or use the **Search** tab to filter by `run_id`.
4. Select a trace to see the sequence of events across the API and Engine.
