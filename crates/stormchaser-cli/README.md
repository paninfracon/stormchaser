# stormchaser-cli

The `stormchaser` command-line client. It:

- logs in through the browser with OIDC and stores the token in a per-URL profile (`stormchaser login`, `stormchaser auth`);
- runs a local workflow file (`stormchaser run`), queues a run of a workflow in a Git repository (`stormchaser runs enqueue`), and lists, watches, approves, rejects, deletes and reads the logs, artifacts and test reports of runs (`stormchaser runs`);
- manages webhooks, event rules, connections and cron schedules;
- lints workflow files offline against the DSL's JSON Schema (`stormchaser lint`) and exports that schema (`stormchaser schema`).

It takes the API URL from `--url` or `STORMCHASER_URL`, a token from `STORMCHASER_TOKEN` or a profile, and a private CA from `--ca-cert` or `STORMCHASER_CA_CERT`.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
