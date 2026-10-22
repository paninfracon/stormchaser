# stormchaser-agent

Helper binary that runners place in step containers and pods. It handles a step's storage and outputs for the Stormchaser File System (SFS):

- `unpark` downloads a storage tarball (or, with `--no-extract`, a single artifact file), checks its SHA-256 hash and extracts it into the step's volume.
- `park` runs beside the step's container (a Kubernetes sidecar or a Docker helper container that shares only the storage volumes), waits for the step to finish, then uploads its storage and publishes its artifacts.
- `run` runs the step's command, collects its test reports, then parks its storage.

The agent bundles the Mozilla root certificates and merges them with the system store, so it works in task images without a CA bundle. It always logs at INFO, because runners read the hashes, artifacts and reports it reports from those lines. Release builds link it statically (musl).

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
