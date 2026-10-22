# stormchaser-opa

Open Policy Agent integration shared by the API, the query service and the engine. It evaluates OPA policies compiled to WebAssembly (with Wasmtime) and chooses the policy a service enforces: a WASM policy at `OPA_WASM_PATH`, else an OPA server at `OPA_URL`, else a built-in copy of the shipped policy (`deploy/opa/policy.rego`, compiled to `policy/stormchaser.wasm` by `scripts/build-opa-policy.sh`). `OPA_DATA_PATH` gives a WASM policy its settings, and `OPA_ENTRYPOINT` names the rule (`stormchaser/allow` by default). A request whose policy check fails is refused. Only a debug build honours `STORMCHASER_AUTHZ_DISABLED=true`; a release build refuses to start with it.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
