# Stormchaser: A workflow engine for event driven and human triggered workflows

## Features

1. ✅ Graph based DSL for defining workflows (NOT YAML!)
2. ✅ Reads workflow definitions and static secrets from Git (SOPS encrypted)
   and syncs to DB for fast execution context
3. ✅ Workflow runtime status and historical audit logs in a Postgres db
   (including granular step state history)
4. ✅ Sequential and parallel workflow steps
5. ✅ Output/Input passing between workflow steps
6. ✅ Distributed workflow execution
7. ✅ Workflow step dispatch to container runtimes (K8s, EKS, ECS etc)
8. 🚧 Workflow step optimization to run multiple steps in one container where
   possible (e.g. small python steps)
9. ✅ Workflow input specification schemas
10. 🚧 Workflow input validation including query ability (API, SQL, AWS, etc)
    to retrieve valid values
11. ✅ Long running workflows
12. ✅ Human interaction steps in workflow (e.g. wait for approval, provide
    inputs, process email response via unauthenticated encrypted links)
13. ✅ Resource management and timeout capabilities
14. ✅ Step restart if execution engine fails
15. ✅ CLI for viewing and scheduling runs
16. ✅ Interactive TUI for real-time monitoring and log viewing
17. ⏳ Pluggable secrets management: First-class SOPS support in Git, plus
    runtime fetching from Vault, AWS Secrets Manager, etc.
18. ✅ Webhook endpoint for receiving events
19. ✅ Authn and Authz: First-class SSO support (OIDC, SAML, OAuth2). For
    environments without a corporate provider, a local SSO provider (e.g.,
    Keycloak, Dex, or Authelia) is recommended for central identity.
20. ✅ Comprehensive log management (built-in or via Grafana Loki)
21. ✅ Execution metrics (run time, failure rates etc)
22. ✅ Steps including "Invoke Webhook", "Send Email", "Request Approval",
    "Wait for Event", "Fail If", "Conditional", "Sequential", "Parallel",
    "Run Container", "Git checkout", "AWS Lambda Invoke"
23. ✅ Shared file system for steps (including ability to read and "park" the
    file system as a tar bundle in object storage with SHA-256 hash
    verification)
24. ✅ Native Junit format test results file ingest
25. ✅ Orchestration engine is event driven state machine using NATS JetStream
    persistent event store and postgres for data
26. ✅ stdin, stdout and environmental variable handling
27. ✅ Extensible step library, including DSL extensibility (e.g. add new step
    type and expand the DSL AST for it)
28. ✅ Mermaid rendering of workflows
29. ✅ Event Rules Engine for mapping, filtering, and transforming events to
    workflows
30. 🚧 Sensors for polling external systems (Jira, GitHub, DB) to emit events
31. ✅ Reusable Workflow Templates and Sub-workflows
32. ✅ CronWorkflows for periodic scheduling (via external systems like
    Kubernetes CronJobs or Ofelia)
33. ✅ Dynamic Parallelism (Map/Reduce) based on runtime input lists
    (including 'max_parallel' batching)
34. 🚧 Step Memoization/Caching to skip execution if inputs/code are unchanged
35. 🚧 Concurrency Limits (global/per-workflow) to prevent resource exhaustion
36. ✅ Advanced Retry Policies with exponential backoff and jitter
37. ✅ Error Handling Hooks (On-Failure/Finally blocks) for resource cleanup
38. ✅ Policy as Code (OPA integration) for workflow execution validation
39. 🚧 Continuous Verification steps with health metric monitoring and automatic
    rollbacks
40. 🚧 Dry-run and Linting mode for workflow validation without side effects
41. ✅ Explicit Artifact Management (S3/GCS/Minio) with SHA-256 hash
    verification and audit trails
42. 🚧 ChatOps integration for Slack/Teams (approvals, status updates)
43. ✅ OpenTelemetry integration for full execution tracing (Jaeger, Honeycomb)
44. 🚧 Environment and Service abstractions for cross-context workflow reuse

## Implementation

1. Rust language project
2. Postgres backend
3. NATS for events and queues
4. Distributed controller with concensus and leader election
5. Leverage generic `tree-sitter-hcl` grammar for DSL parsing and editor
   support

## Out of scope

1. Web UI (for now)
