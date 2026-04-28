# Future TODOs

## Reliability & Crash Recovery

- **Crash Handling**: On startup, an engine instance should query the database for workflows in the `resolving` state that were owned by dead orchestrators (e.g., via fencing tokens or heartbeat timeouts) and resume their resolution process.

## Integrations

- **Sensors (Polling)**: Implement background polling mechanisms for external systems (Jira, GitHub, Databases) to emit NATS events automatically for event-driven workflows. *Note: Webhooks are the primary integration point currently supported.*

## Enhancements

- **Step Memoization/Caching**: The ability to skip execution if inputs or step code haven't changed since the last successful run.
- **Native WASM on Kubernetes**: Integrate `https://kwasm.sh/` for executing Wasm steps natively within the Kubernetes runner environment.
