# stormchaser-dsl

Parser for the Stormchaser workflow DSL (`.storm` files, HCL syntax). It turns HCL into the workflow types in `stormchaser-model`, applies step libraries to the steps that use them (spec merge, params, retry, timeout, `allow_failure`), enforces parser limits such as file size and nesting depth, and builds the JSON Schema of a workflow's inputs. It also converts between JSON Schema and its HCL form, in both directions.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
