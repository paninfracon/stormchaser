# stormchaser-client

A typed client for the Stormchaser API and query service, used by the CLI, the TUI and the web UI's server. Requests and responses are the types in `stormchaser_model::api`, so every client reads and writes them as the API does. Streams (run lists, statuses, logs, form hydration) come back as server-sent events.
