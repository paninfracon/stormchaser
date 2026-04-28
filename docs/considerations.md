# Stormchaser Project Considerations

This document outlines identified gaps, errors, omissions, and risks based on the current architectural and DSL documentation.

## 1. Architectural & Operational Risks

### Github duplicate webhooks

If you are using JetStream, it has a built-in deduplication window. When your webhook gateway receives the payload, extract the X-GitHub-Delivery header (the unique GUID GitHub assigns to every webhook event) and set it as the Nats-Msg-Id header on your published message. JetStream will automatically drop duplicate messages with the same ID within a configurable time window. (If header doesn't exist use some other appropriate nonce)

### Diagnosability

Build a native trace command into your CLI. When a workflow fails, the developer should be able to type engine trace <GUID>. The CLI should hit your API server, pull the exact state transition history from Postgres, pull the terminal logs from the Loki proxy, and render a clean, color-coded timeline in their terminal. Show them the exact line of HCL that failed and the exact container exit code. Correlation IDs should be on all OTEL log messages too

### OPA bootstrap

Utilize the ability of OPA to compile rulesets to WASM and use a rust wasm engine to load them. Provide a default precanned ruleset, allow external OPA API and also allow custom wasm OPA to be loaded instead of the default set

## 2. DSL & Feature Omissions

* (None)

## 3. Documentation Errors & Inconsistencies

### Examples

Take it a step further and embed the documentation directly into the CLI. If a developer types engine explain step docker-build, it should dump the HCL schema and a working example directly to standard out. Bring the docs to where their hands already are.

## 4. Security Risks

* (None)
