# stormchaser-tls

TLS setup shared by the Stormchaser services. It builds `rustls` client and server configurations from certificate files, reloads them when the files change, and supports client certificates. The `listener` feature adds an HTTPS listener for Axum services, used by the API, the query service and the web UI. `parse_switch` reads on/off settings such as `STORMCHASER_LISTEN_TLS` and `STORMCHASER_DB_SSL` and accepts only `true` or `false`.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
