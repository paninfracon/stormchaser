# NATS Schema Improvements

* Ensure we are conformant to the CloudEvents spec for all our NATS messages <https://github.com/cloudevents/spec>
* Use the existing OCI container repository backend as a schema version repository
* Expect to use the schemas in the schema version repository but cache the parsed version locally for performance
* Use these guidelines when versioning messages in NATS

    1. Subject-Based Versioning (Routing Level) This is the most critical layer. By including a version token in your subject, you allow consumers to decide exactly which versions of a message they want to "listen" to.
       Pattern: `ORDERS.<version>.<event_type>`
       Example: `ORDERS.v1.created` vs. `ORDERS.v2.created`
       Selective Consumption: A new service can subscribe to `ORDERS.v2.*`, while an old legacy service stays on `ORDERS.v1.*`.
       Stream Binding: You can create a JetStream that listens to `ORDERS.*.*` to capture every version for auditing, or separate streams for major versions.

    2. Header-Based Metadata (Discovery Level)
       Use NATS Headers to store schema metadata. This allows consumers to inspect the version without having to deserialize the entire payload (which is expensive).
       Nats-Msg-Schema-Version: The specific version (e.g., 1.0.2).
       Content-Type: Use standard mime-types like application/json or application/x-protobuf.
       Schema ID: If using a schema registry, include a Schema-ID header.
* Document how to use: NATS allows for Subject Mapping, which is a powerful way to handle migrations transparently. If you decide to move from ORDERS.v1 to ORDERS.v2, you can configure the NATS server to automatically re-route
