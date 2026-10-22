# Stormchaser: A workflow engine for event driven and human triggered workflows

## Features

✅ works today. 🚧 partly built. ⏳ planned.

1. ✅ Graph based DSL for defining workflows (NOT YAML!)
2. ✅ Reads workflow definitions and static secrets (SOPS encrypted) from Git,
   through a local clone cache, and stores each run's resolved definition in
   the database
3. ✅ Workflow runtime status and historical audit logs in a Postgres db
   (including granular step state history)
4. ✅ Steps that run one after another or side by side, ordered by `next`
   (the `Parallel` and `Sequential` group step types are planned)
5. ✅ Output/Input passing between workflow steps
6. ✅ Distributed workflow execution
7. ✅ Workflow step dispatch to container runtimes: Docker and Kubernetes
   (any conformant cluster, such as EKS). ECS is not supported
8. ⏳ Workflow step optimization to run multiple steps in one container where
   possible (e.g. small python steps). `shared` affinity groups pin their steps
   to one runner today
9. ✅ Workflow input specification schemas
10. ✅ Workflow input validation including queries (API, SQL, AWS Cloud
    Control) to retrieve valid values, run by the query service
11. ✅ Long running workflows
12. ✅ Human interaction steps in workflow (wait for approval, provide
    inputs, approve or reject from sealed, expiring links in an email, which
    open a confirmation page)
13. ✅ Resource management and timeout capabilities
14. 🚧 Recovery when an engine fails: events are delivered again to another
    replica, lost dispatches are dispatched again, and `JQ` and `JinjaRender`
    steps run again. Other built-in steps of a dead engine are failed, since
    they may have had their effect
15. ✅ CLI for viewing and scheduling runs
16. ✅ Interactive TUI (real-time monitoring, log viewing, Admin lists, Reporting) and Web UI (Admin interface, Reporting, Import/Export features)
17. 🚧 Pluggable secrets management: SOPS-encrypted files in Git and runtime
    fetching from HashiCorp Vault work, behind allow-lists. AWS Secrets
    Manager and other backends are planned
18. ✅ Webhook endpoint for receiving events (GitHub signatures, bearer
    tokens, Standard Webhooks signatures), with repeated deliveries ignored
19. ✅ Authn and Authz: SSO through OIDC. SAML and other providers work
    through an OIDC broker such as Dex or Keycloak, which is also the
    recommended local provider where there is no corporate one. Authorization
    is OPA policy on every request and object
20. ✅ Step log collection into Grafana Loki or Elasticsearch, read back
    through the API
21. ✅ Execution metrics (run time, failure rates etc)
22. ✅ Steps including `WebhookInvoke`, `RestApi`, `SqlExecute`, `JQ`,
    `JinjaRender`, `EmailSend`, `TestReportEmail`, `Approval`, `Wait`,
    `RunContainer`, `RunK8sJob`,
    `GitCheckout`, `LambdaInvoke`, `Wasm`, `SlackMessage`, `TeamsMessage` and
    the Terraform steps. Any step can carry a `condition`
23. ✅ Shared file system for steps (including ability to read and "park" the
    file system as a tar bundle in object storage with SHA-256 hash
    verification)
24. ✅ Native Junit format test results file ingest
25. ✅ Orchestration engine is event driven state machine using NATS JetStream
    persistent event store and postgres for data
26. ✅ stdin, stdout and environmental variable handling
27. ✅ Extensible steps: step libraries with parameters, and new step types
    registered as WASM modules
28. ⏳ Mermaid rendering of workflows (the repository's diagrams are rendered
    by hand with `mermaid-cli`)
29. ✅ Event Rules Engine for mapping, filtering, and transforming events to
    workflows
30. ⏳ Sensors for polling external systems (Jira, GitHub, DB) to emit events
31. ✅ Reusable workflows: `include` inlines other workflows and their step
    libraries
32. 🚧 [Model Context Protocol (MCP) Server](mcp-server.md) for exposing the API to AI agents
    (tool calls do not yet forward the caller's token, so only the API's shape can be explored)
33. ✅ CronWorkflows for periodic scheduling (via external systems like
    Kubernetes CronJobs, or any scheduler that can call the trigger URL)
34. ✅ Dynamic Parallelism (Map/Reduce) based on runtime input lists
    (including 'max_parallel' batching)
35. ⏳ Step Memoization/Caching to skip execution if inputs/code are unchanged
36. 🚧 Concurrency Limits: per-run quotas (`max_concurrency`, CPU, memory)
    under operator ceilings work. Global and per-workflow limits are planned
37. ✅ Step retry policies (`retry` block) with fixed, linear or exponential
    backoff and a maximum delay. No jitter
38. 🚧 Error handling: `allow_failure` works. `on_failure` and `finally`
    blocks are parsed but the engine does not run them yet
39. ✅ Policy as Code (OPA integration) for workflow execution validation
40. ⏳ Continuous Verification steps with health metric monitoring and automatic
    rollbacks
41. ✅ Offline Linting mode and JSON schema validation for workflows without side effects
42. ✅ Explicit Artifact Management (S3/GCS/Minio) with SHA-256 hash
    verification and audit trails
43. 🚧 ChatOps: `SlackMessage` and `TeamsMessage` steps send messages.
    Approving from Slack or Teams is planned
44. ✅ OpenTelemetry traces and metrics over OTLP, to any OTLP backend
    (Tempo, Jaeger, Honeycomb)
45. ⏳ Environment and Service abstractions for cross-context workflow reuse

## Implementation

1. Rust language project
2. Postgres backend
3. NATS for events and queues
4. Several engine replicas, splitting events by shard, with a leader elected
   through a Postgres advisory lock to run the background workers. No
   consensus protocol of its own
5. DSL parsing with `hcl-rs`. A `tree-sitter-hcl` grammar for editor support
   is planned
