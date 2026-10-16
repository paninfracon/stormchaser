# stormchaser-agent

Runner-side helper for Stormchaser. Manages storage "parking" for the Stormchaser File System (SFS) and publishes artifacts from a helper container or pod sidecar that the step's own code does not share (`park`, `unpark`), and wraps a step's command to collect its test reports (`run`).

---

## About Stormchaser

Stormchaser is a robust, distributed workflow engine for event-driven and human-triggered workflows. Built in Rust for performance and reliability, it utilizes a graph-based DSL, NATS JetStream for event messaging, and PostgreSQL for state management.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
