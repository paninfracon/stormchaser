# stormchaser-tui

Terminal UI for Stormchaser, built with Ratatui. It lists and watches runs and their steps, streams logs, shows artifacts and test results, handles pending approvals, starts runs with forms built from a workflow's input schema (filled in by the query service), and manages connections, webhooks, event rules and cron schedules. It logs in through the browser with OIDC.

It takes the API URL from `--url` or `STORMCHASER_URL` (default `http://localhost:3000`) and the query service URL from `--query-url` or `STORMCHASER_QUERY_URL` (default `http://localhost:3001`).

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
