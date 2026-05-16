# NATS Schema Improvements (Completed)

All foundational NATS messaging infrastructure has been modernized and deployed.

* **CloudEvents Conformant:** All NATS messages now strictly conform to the CloudEvents specification (`cloudevents::Event`), ensuring standardized routing and data representation.
* **OCI Schema Registry:** We are now utilizing the existing OCI container repository backend as a schema version repository.
* **Local Caching:** Parsed schemas from the OCI registry are cached locally (`schema_cache.rs`) to eliminate network bottlenecks during event validation.
* **Subject-Based Versioning:** NATS subjects have been updated to include routing versions to allow selective consumption and easy stream binding (e.g., `stormchaser.v1.run.queued`).
* **Header-Based Metadata:** CloudEvent standard headers (such as `Content-Type: application/cloudevents+json`) and specific schema identifiers are injected directly into the NATS Headers, allowing consumers to inspect payloads without expensive deserialization.
* **Subject Mapping:** Migrations can now be handled transparently using native NATS Subject Mapping (e.g., mapping `.v1.` to `.v2.`).
