# Stormchaser Kubernetes Environments

Stormchaser supports three primary deployment models: **Docker Only**,
**Hybrid Local (Docker + MicroK8s)**, and **Full Kubernetes (Helm)**. The
Docker-only setup is described in [Local Development with Docker](local_dev_docker.md).
Section 5 lists every environment variable the services read.

---

## 1. Quick Start: Full Stack with MicroK8s

This is the fastest way to get the full stack running on Kubernetes on your
local machine, complete with automated log management and monitoring
(Prometheus/Grafana).

### Quick Start Prerequisites

- **MicroK8s**: Install via `snap install microk8s --classic`. The Kubernetes
  runner is built against the Kubernetes 1.32 API, so use a cluster running
  1.32 or newer.
- **Enable Addons**:

  ```bash
  microk8s enable dns rbac storage ingress
  ```

- **Helm**: Ensure `helm` is installed locally.
- **Docker**, to build the images, and Python 3 with `passlib[bcrypt]`, to
  hash the Dex persona passwords.

> **This is an evaluation environment.** It deploys single-node PostgreSQL,
> NATS and MinIO alongside Stormchaser. PostgreSQL is not encrypted at rest,
> so do not store real credentials in it. NATS requires generated passwords
> but has no TLS and no clustering. Before relying on it, read
> [Encryption at rest](security.md#5-encryption-at-rest) and
> [NATS: Quickstart and Production](nats.md).

### Step 1: Run the Deployment Script

`scripts/setup.sh --mode microk8s` builds the Stormchaser images and imports
them into MicroK8s, enables the `dns`, `rbac`, `storage` and `ingress` addons,
deploys the umbrella chart (`deploy/charts/stormchaser`) into the
`stormchaser` namespace, deploys Dex (`deploy/dex-k8s/`), and registers the
bundled MinIO as the default SFS storage. The chart generates the services'
TLS certificate, with its own CA, and the database and JWT secrets.

```bash
# Execute the deployment script
./scripts/setup.sh --mode microk8s
```

### Step 2: Accessing the Infrastructure

To access the core services from your host machine, you will need to set up
port forwarding.

- **Orchestration API**: Access the primary REST API.

  ```bash
  kubectl port-forward -n stormchaser \
    svc/stormchaser-stormchaser-orchestration-api 3000:3000
  ```

  The API serves HTTPS with a certificate from the CA the chart generated. Save
  that CA and pass it to the CLI or TUI with `--ca-cert` (or
  `STORMCHASER_CA_CERT`), or to curl with `--cacert`:

  ```bash
  kubectl get secret stormchaser-tls-certs -n stormchaser \
    -o jsonpath='{.data.ca\.crt}' | base64 -d > stormchaser-ca.crt
  stormchaser --url https://localhost:3000 --ca-cert stormchaser-ca.crt login
  ```

- **Dex (Authentication)**: Access the OIDC provider.

  ```bash
  kubectl port-forward -n stormchaser svc/dex 5556:5556
  ```

  *Note: The default `stormchaser-admin@paninfracon.net` password is
  dynamically generated during setup. Retrieve it using:*

  ```bash
  kubectl get secret dex-admin-secret -n stormchaser \
    -o jsonpath='{.data.password}' | base64 -d
  ```

By default, the full stack includes a pre-wired monitoring solution:

- **Grafana**: Access the dashboard to view system metrics, logs, and traces.

  ```bash
  kubectl port-forward -n stormchaser svc/stormchaser-grafana 3001:80
  ```

  Open `http://localhost:3001`.
- **Prometheus**: View raw metrics and alerts.

  ```bash
  kubectl port-forward -n stormchaser svc/stormchaser-prometheus-server 9090:80
  ```

- **Tempo**: Query distributed traces.

  ```bash
  kubectl port-forward -n stormchaser svc/stormchaser-tempo-query 3200:3200
  ```

### Step 3: Out-of-the-Box Observability Features

- **Metrics**: The Orchestration API and Engine automatically push OTLP metrics
  to the bundled Grafana Alloy instance, which forwards them to Prometheus.
- **Tracing**: Full distributed tracing is enabled via Grafana Tempo. Every state
  transition and API call is recorded and tagged with the `run_id`, allowing
  for end-to-end visualization of workflow execution.
- **Database Observability**: SQL queries are automatically instrumented and
  emitted as tracing spans, providing visibility into database performance
  and bottlenecks.
- **Logs**: Grafana Alloy automatically scrapes logs from all Stormchaser
  workflow pods and pushes them to Loki. Grafana is pre-configured with Loki,
  Prometheus, and Tempo datasources.

---

## 2. Hybrid Local Development (MicroK8s)

This is the recommended mode for **developing** Stormchaser itself. It uses
Docker Compose to run the control plane (for faster iteration) and MicroK8s to
run only the `stormchaser-runner-k8s`.

### Hybrid Mode Prerequisites

- **Docker** and **Docker Compose**
- **MicroK8s**. The script enables the `dns`, `rbac`, `storage` and `ingress`
  addons if they are off.
- **Helm**

### Setup

The local setup script handles certificate generation, bridging the networks,
and runner deployment.

```bash
# Start in hybrid mode (the default mode is docker)
./scripts/setup.sh --mode hybrid

# Full reset and restart
./scripts/setup.sh --cleanup --mode hybrid
```

Hybrid mode runs the compose project `stormchaser-hybrid` on its own host
ports so it can sit beside a Docker-mode stack: API 3001, PostgreSQL 5433,
NATS 4223, Dex 5557, Loki 3101, MinIO 9002 and 9003, OPA 8182, web UI 3004.
The control-plane ports are published on all interfaces so the runner in
MicroK8s can reach NATS and MinIO.

### Hybrid Architecture

- **Docker Compose**: Runs core infrastructure (Postgres, NATS, Dex, OPA,
  MinIO, Loki, Tempo, Prometheus, Grafana, Alloy) and the orchestration
  services (API, query service, engine, web UI).
- **MicroK8s**: Runs `stormchaser-runner-k8s` (Helm release
  `stormchaser-runner` in the `default` namespace), Grafana Alloy for step
  logs, and the workflow steps as Kubernetes Jobs in the runner's step
  namespace.

You can interact with your local MicroK8s cluster using the generated
kubeconfig:

```bash
export KUBECONFIG=.tmp/kubeconfig
kubectl get pods
```

---

## 3. Deployment to an Existing Kubernetes Cluster

When you are ready to deploy Stormchaser into an existing cluster
(e.g., EKS, GKE, AKS, k3s), we provide Helm charts located in the
`deploy/charts/` directory.

Stormchaser is split into two primary charts to allow for flexible
architectures (e.g., running the Orchestration engine in a central cluster,
and the Runners in isolated worker clusters).

### Cluster Prerequisites

- A running Kubernetes cluster.
- `helm` and `kubectl` installed and configured to communicate with your
  cluster.
- External dependencies provisioned (PostgreSQL Database, NATS JetStream
  cluster). PostgreSQL must be encrypted at rest and reached over TLS:
  Stormchaser stores workflow secrets and connection credentials in it and
  does not encrypt them itself; see
  [Encryption at rest](security.md#5-encryption-at-rest) for what your
  database administrator needs to provide. NATS must require authentication, give runners a separate
  restricted user, use TLS, and be unreachable from workflow step pods; see
  [NATS: Quickstart and Production](nats.md) for the exact requirements.
- A log backend of your own (Loki, or Elasticsearch). The bundled Loki,
  Tempo, Prometheus and Grafana are for the quickstart. Step logs hold
  whatever a workflow prints, so the backend must require authentication,
  be unreachable from workflow step pods, and keep logs no longer than your
  retention policy allows; point `LOKI_URL` (or the Elasticsearch settings) at
  it.

### Step 1: Provision the Namespace and Secrets

First, create a namespace for Stormchaser and populate the necessary secrets
(TLS certificates, Database credentials, etc.).

```bash
kubectl create namespace stormchaser

# The database URL. Use a host name the database's certificate carries:
# the services connect with sslmode=verify-full unless told otherwise.
kubectl create secret generic stormchaser-db-secret \
  --namespace stormchaser \
  --from-literal=DATABASE_URL="postgres://user:password@host:5432/db"

# NATS credentials: one Secret for the control plane, one for runners
kubectl create secret generic nats-control \
  --namespace stormchaser \
  --from-literal=user=stormchaser \
  --from-literal=password='<control password>'
kubectl create secret generic nats-runner \
  --namespace stormchaser \
  --from-literal=user=stormchaser-runner \
  --from-literal=password='<runner password>'

# The services' certificate, its key, and the CA that signed it. The API, the
# query service and the web tier serve HTTPS with it, so it must name their
# Services (<release>-stormchaser-orchestration-api, -query and -web).
kubectl create secret generic stormchaser-tls-certs \
  --namespace stormchaser \
  --from-file=ca.crt=path/to/ca.crt \
  --from-file=tls.crt=path/to/tls.crt \
  --from-file=tls.key=path/to/tls.key
```

### Step 2: Install the Orchestration Chart

The orchestration chart deploys the Control Plane (API and Engine).

Review the `deploy/charts/stormchaser-orchestration/values.yaml` file. You will
need to override values to point to your external dependencies (NATS, Postgres,
OIDC provider).

```bash
helm install stormchaser-orchestration \
  ./deploy/charts/stormchaser-orchestration \
  --namespace stormchaser \
  --set global.database.existingSecret="stormchaser-db-secret" \
  --set global.nats.url="tls://my-nats-cluster:4222" \
  --set global.nats.authSecret="nats-control" \
  --set global.jwt.existingSecret="stormchaser-jwt" \
  --set global.oidc.issuer="https://my-identity-provider/dex" \
  --set global.oidc.existingSecret="stormchaser-oidc-client"
```

The chart refuses to render without a database URL, and the API cannot sign
in anyone without the JWT secret (`jwt-secret`, 32+ random characters) or
complete a browser login without the OIDC client secret (`client-secret`).
Create those Secrets first. Also consider:

- `global.nats.apiAuthSecret` and `global.nats.queryAuthSecret`: narrower NATS
  users for the API and the query service (see [NATS](nats.md)).
- `global.approvalLinks.existingSecret`: the key that seals approval links.
  Without it, approval emails carry no links.
- `global.ssrfAllowedHosts` and `global.aws.allowedRoleArns`: allow-lists,
  empty by default, for internal hosts and IAM roles workflows may use.
- `global.tls.listen=false` where an ingress or mesh terminates TLS in front
  of the services and the network behind is trusted.
- `api.opa.sidecar.settings`: the shipped policy's settings, including
  `admins` and `adminGroups` (see [OPA RBAC](opa-rbac.md)).

### Step 3: Install the K8s Runner

The runner chart deploys the execution agent responsible for managing
Kubernetes Jobs for workflow steps. It needs to communicate with the NATS
cluster.

Review the `deploy/charts/stormchaser-runner-k8s/values.yaml` file to configure
NATS and runner behavior.

```bash
helm install stormchaser-runner ./deploy/charts/stormchaser-runner-k8s \
  --namespace stormchaser \
  --set config.natsUrl="tls://my-nats-cluster:4222" \
  --set config.natsAuthSecret="nats-runner" \
  --set rbac.create=true
```

*Note: Workflow steps never run in the runner's own namespace. The chart creates
a separate step namespace (`rbac.targetNamespace`, by default
`<release namespace>-steps`, here `stormchaser-steps`) and a Role and
RoleBinding there so the runner can spawn and monitor Jobs and Pods. The runner
refuses to start if the step namespace is unset or equals its own.*

### Step 4: Verify Deployment

Check that the Control Plane and Runner pods are healthy:

```bash
kubectl get pods -n stormchaser
```

You should see:

- `stormchaser-orchestration-api-...`
- `stormchaser-orchestration-engine-...`
- `stormchaser-runner-k8s-...`

At this point, you can configure your CLI to point to the external API's
LoadBalancer or Ingress endpoint to submit workflows!

---

## 4. Teardown and Cleanup

If you need to reset your local development environment to a completely clean
state, follow these steps to remove all Docker containers, clean Cargo build
artifacts, and reset MicroK8s.

> [!WARNING]
> These commands are destructive and will completely wipe all local Docker and
> MicroK8s state. This includes tearing down **all** containers, volumes, and
> Kubernetes resources, even those not related to Stormchaser (i.e., other
> tenants, projects, or workloads running in your local Docker or MicroK8s
> engines). Proceed with caution if you use these engines for other projects.

```bash
# 1. Stop and remove the setup script's compose projects and their volumes
docker compose -p stormchaser-docker --profile "*" down -v --remove-orphans
docker compose -p stormchaser-hybrid --profile "*" down -v --remove-orphans

# 2. Prune Docker system (removes stopped containers, dangling images, build cache)
docker system prune -f

# 3. Clean Cargo build artifacts
cargo clean

# 4. Reset MicroK8s (requires elevated privileges)
sudo microk8s reset

# 5. Remove generated local files (they are regenerated by setup.sh)
rm -rf .tmp scripts/.tmp
rm -f .env deploy/dex/config.generated.yaml deploy/dex/credentials.generated
```

---

## 5. Configuration Reference

This section lists every environment variable the Stormchaser binaries read, which
service reads it, its default and what it does. It was taken from the code on
2026-10-19; where this table and the code disagree, the code is right.

"Helm:" names the chart value that sets a variable, where a chart sets it. Values are
in `deploy/charts/stormchaser-orchestration` unless the runner charts are named. A
variable with no Helm value can still be set, but the chart has no knob for it. Ports
are fixed in code: the API listens on 3000, the Docker runner's health server on 8081
and the Kubernetes runner's on 8080.

Switches read with `stormchaser_tls::parse_switch` accept only `true` or `false` (any
case) and stop the service on anything else. Other switches are noted one by one. Most
numeric limits fall back to their default when the value does not parse or is 0.

### Shared by several services

| Variable | Read by | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | API, engine, query | none, **required** | PostgreSQL connection URL. The service will not start without it. Helm: `global.database.url` (into a Secret) or `global.database.existingSecret`. |
| `STORMCHASER_DB_SSL` | API, engine, query | unset | `true` or `false` only. `true`: verified TLS (`verify-full`) whatever the URL says. `false`: the URL's `sslmode`, or no TLS. Unset: the URL's `sslmode`, or verified TLS if the URL names none. Helm: `global.database.ssl` (chart default `true`). |
| `STORMCHASER_DB_MAX_CONNECTIONS` | API, engine, query | `10` | Size of the service's connection pool. A value that is not a whole number above 0 stops the service. |
| `STORMCHASER_MIGRATE_ON_START` | API, engine | `false` | `true` or `false` only. `true`: the service applies the database migrations itself at start-up, for running a service on its own in development. Otherwise `stormchaser-engine migrate` applies them (the Helm chart runs it in the engine pod's init container; compose has a `migrate` service) and the service waits for it. |
| `STORMCHASER_SCHEMA_WAIT` | API, engine | `5m` | How long the service waits at start-up for the database schema to be migrated before it gives up and exits. |
| `TLS_CA_CERT_PATH` | API, engine, query, web | unset | CA bundle trusted for the database, NATS, OPA and calls between services, as well as the system roots. Helm: `global.tls.caCertPath`, set only when `global.tls.enabled`. |
| `TLS_CERT_PATH` | API, engine, query, web | `/etc/engine/certs/tls.crt` | The service's certificate: served by its listener, and offered as a client certificate. Helm: `global.tls.certPath` (`/etc/stormchaser/certs/tls.crt`), set only when `global.tls.enabled`. |
| `TLS_KEY_PATH` | API, engine, query, web | `/etc/engine/certs/tls.key` | Key for `TLS_CERT_PATH`. Helm: `global.tls.keyPath`. |
| `TLS_SERVER_NAME` | API, engine, query | unset | Server name to verify on outbound TLS connections in place of the host name. |
| `STORMCHASER_LISTEN_TLS` | API, query, web | `true` | `true` or `false` only. `false` serves plain HTTP, for use behind something that terminates TLS. With `true` a service that cannot load its certificate refuses to start. Helm: `global.tls.listen`. |
| `NATS_URL` | API, engine, query, both runners | `nats://localhost:4222` | NATS server. Helm: `global.nats.url`; runner charts: `config.natsUrl`. |
| `NATS_CREDS_FILE` | API, engine, query, both runners | unset | NATS `.creds` file. Used in preference to the variables below. |
| `NATS_USER` | API, engine, query, both runners | unset | NATS user, used with `NATS_PASSWORD`. Also names the reply inbox (`_INBOX_<user>`). Helm: from `global.nats.authSecret` (`apiAuthSecret`, `queryAuthSecret` override it); runner charts: `config.natsAuthSecret`. |
| `NATS_PASSWORD` | API, engine, query, both runners | unset | NATS password. Helm: as `NATS_USER`. |
| `NATS_TOKEN` | API, engine, query, both runners | unset | NATS token, used when no creds file or user and password is set. With none of the three the service connects without credentials and logs a warning. |
| `NATS_CA_FILE` | API, engine, query, both runners | unset | Extra root certificate for the NATS connection. |
| `NATS_TLS_REQUIRED` | API, engine, query, both runners | off | `true` (any case) or `1` refuses a NATS server without TLS. |
| `NATS_INBOX_PREFIX` | API, engine, query, both runners | `_INBOX_<NATS_USER>`, or `_INBOX` | Subject prefix for replies. |
| `STORMCHASER_TOTAL_SHARDS` | API, engine, both runners | `1` | Number of shards runs are spread over. Must be the same everywhere. |
| `OPA_URL` | API, engine, query | unset | OPA server to ask for decisions. Empty counts as unset. Helm: `global.opa.url` (chart default, the sidecar at `http://127.0.0.1:8181/...`). |
| `OPA_WASM_PATH` | API, engine, query | unset | Policy compiled to WebAssembly, used in preference to `OPA_URL`. With neither set, the built-in policy is enforced. Helm: `query.opaWasmPath` (query only). |
| `OPA_DATA_PATH` | API, engine, query | unset | JSON `data` document for a WebAssembly policy. Ignored with `OPA_URL`. |
| `OPA_ENTRYPOINT` | API, engine, query | `stormchaser/allow` | Policy rule to evaluate. |
| `STORMCHASER_AUTHZ_DISABLED` | API, engine, query | `false` | `true` or `false` only. `true` enforces no policy. A release build refuses to start with it. |
| `LOKI_URL` | API, engine, query, runner-docker | unset | API, engine, query: Loki base URL to read run logs from. Docker runner: Loki push URL given to the Docker `loki` logging driver for step containers. Helm: `global.loki.url`. |
| `ELASTICSEARCH_URL` | API, engine, query | unset | Elasticsearch log backend, used when `LOKI_URL` is unset and `ELASTICSEARCH_INDEX` is set. |
| `ELASTICSEARCH_INDEX` | API, engine, query | unset | Index for `ELASTICSEARCH_URL`. |
| `RUST_LOG` | API, engine, query, both runners | per service, see description | Log filter in `tracing` `EnvFilter` syntax. Defaults: API `stormchaser_api=info,tower_http=info`, engine `stormchaser_engine=info`, runners `stormchaser_runner_docker=info` and `stormchaser_runner_k8s=info`. The query service has no default of its own, so unset logs errors only. Helm: `api.rustLog`, `engine.rustLog`, `query.rustLog`; runner charts: `config.rustLog`. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | API, engine | unset | OTLP gRPC endpoint. Set, traces and metrics are exported there; unset, nothing is exported. Helm: `global.otel.endpoint`. |
| `OTEL_SERVICE_NAME` | API, engine | `stormchaser-api`, `stormchaser-engine` | Service name on exported telemetry. |
| `STORMCHASER_JWT_SECRET` | API, query | none, **required** in release builds | Key that signs and checks the API's own session tokens. Empty is an error. A debug build falls back to a built-in key. Helm: `global.jwt.existingSecret` (API only). |
| `OIDC_ISSUER` | API, query | unset | Identity provider issuer URL, used to fetch its keys (`<issuer>/keys`). OIDC is on only when this, `OIDC_CLIENT_ID` and `OIDC_CLIENT_SECRET` are all set. Helm: `global.oidc.issuer`. |
| `OIDC_EXTERNAL_ISSUER` | API, query | `OIDC_ISSUER` | Issuer as browsers and tokens see it, when it differs from the internal URL. Helm: `global.oidc.externalIssuer`. |
| `OIDC_CLIENT_ID` | API, query | unset | OIDC client ID. Helm: `global.oidc.clientId`. |
| `OIDC_CLIENT_SECRET` | API, query | unset | OIDC client secret. Helm: `global.oidc.clientSecret` or `global.oidc.existingSecret` (API only). |
| `STORMCHASER_OIDC_REQUIRE_EMAIL_VERIFIED` | API, query | `true` | `false`, `FALSE` or `0` accepts a provider token whose email is not marked verified. Anything else keeps the check. |
| `STORMCHASER_SSRF_ALLOWED_HOSTS` | API, engine, query | empty | Comma-separated hosts that workflow-driven requests may reach although they resolve to internal addresses. An entry is a host name, an IP, or `*.suffix`. Helm: `global.ssrfAllowedHosts`. |
| `STORMCHASER_AWS_ALLOWED_ROLE_ARNS` | API, engine, query | empty | Comma-separated role ARNs that may be assumed; a trailing `*` matches a prefix. Empty refuses every assume-role. Helm: `global.aws.allowedRoleArns`. |
| `STORMCHASER_APPROVAL_LINK_KEY` | API, engine | unset | Key that seals approve and reject links in approval emails; at least 32 characters, the same on both. Unset or shorter: no links are sent or accepted. Helm: `global.approvalLinks.existingSecret`. |
| `STORMCHASER_RUN_MAX_TIMEOUT` | API, engine | `24h` | Longest a run may last. A run or API override that asks for more gets this. |
| `STORMCHASER_GIT_ALLOWED_SCHEMES` | API, engine, query | `https,ssh` | Comma-separated URL schemes repositories may be cloned over. |
| `STORMCHASER_GIT_MAX_CLONE_MB` | API, engine, query | `1024` | Most data, in MiB, one clone or fetch may download. |
| `STORMCHASER_GIT_CLONE_TIMEOUT_SECS` | API, engine, query | `300` | Longest one clone or fetch may spend downloading. |
| `STORMCHASER_GIT_CACHE_REFRESH_SECS` | API, engine, query | `60` | How long a cached branch or tag is used before it is fetched again. A full commit ID is never refetched. |
| `STORMCHASER_JQ_TIMEOUT_SECS` | engine, query | `10` | Seconds a jq program may run. |
| `STORMCHASER_JQ_MAX_MEMORY_MB` | engine, query | `512` | Memory a jq program may use. |
| `STORMCHASER_EXPR_MAX_COMPLEXITY` | API, engine, query | `8192` | Complexity limit for one HCL expression or template string. Text over it is refused before parsing. |
| `STORMCHASER_DSL_MAX_COMPLEXITY` | every binary that parses workflow files | `16384` | Complexity limit for a whole workflow file. |
| `STORMCHASER_STATE_ENCRYPTION_KEY` | both runners | unset | Key the runner encrypts the step state it keeps on containers or Kubernetes objects with. Unset, that state is stored in plain text. The Docker runner treats an empty value as unset; the Kubernetes runner does not. Helm: runner charts' `config.stateEncryptionSecret`. |
| `RUNNER_ID` | both runners | `<kind>-<HOSTNAME>` | Runner's identity. Unset, it is built from `HOSTNAME` (or the kernel host name) so a restarted runner comes back as itself. Helm: runner-k8s `config.runnerId`. |
| `HOSTNAME` | both runners | kernel host name | Used to build the runner ID. The Docker runner also inspects its own container by this name to find its network when `STORMCHASER_STEP_NETWORK` is unset. |
| `STORMCHASER_ALLOW_PRIVILEGED` | both runners | `false` | `true` (any case) or `1` allows steps that ask for privileged mode. Helm: runner charts' `config.allowPrivileged`. |
| `STORMCHASER_AGENT_IMAGE` | both runners | Docker: `stormchaser-agent:v1`; Kubernetes: unset | Image of the agent that parks storage and publishes artifacts beside a step. On the Kubernetes runner, unset means no agent. Helm: runner-docker `config.agentImage`; runner-k8s `global.agent.image.repository` and `.tag`. |
| `STORMCHASER_STEP_DEFAULT_CPU` | both runners | `1` | CPU a step gets when it asks for none. Helm: runner charts' `config.stepLimits.defaultCpu`. |
| `STORMCHASER_STEP_DEFAULT_MEMORY` | both runners | `2Gi` | Memory a step gets when it asks for none. Helm: `config.stepLimits.defaultMemory`. |
| `STORMCHASER_STEP_MAX_CPU` | both runners | `8` | Most CPU a step may ask for; a step asking for more is refused. Helm: `config.stepLimits.maxCpu`. |
| `STORMCHASER_STEP_MAX_MEMORY` | both runners | `16Gi` | Most memory a step may ask for. Helm: `config.stepLimits.maxMemory`. |
| `STORMCHASER_STEP_DEFAULT_TIMEOUT` | both runners | `24h` | Run time for a step that sets no `timeout`; capped at the maximum. Helm: `config.stepLimits.defaultTimeout`. |
| `STORMCHASER_STEP_MAX_TIMEOUT` | both runners | `24h` | Longest `timeout` a step may set. Helm: `config.stepLimits.maxTimeout`. |

Libraries the services use read some variables of their own: the AWS SDK reads
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_DEFAULT_REGION` and the rest of the
standard AWS set, the Docker runner's Docker client reads `DOCKER_HOST`, the
Kubernetes clients (API cron, runner-k8s) read `KUBECONFIG` outside a cluster, and the
web tier's Leptos runtime reads `LEPTOS_SITE_ADDR`, `LEPTOS_SITE_ROOT` and
`LEPTOS_OUTPUT_NAME` (set in `Dockerfile.web`; the site listens on `0.0.0.0:3000`
there).

### API (`stormchaser-api`)

| Variable | Default | Description |
|---|---|---|
| `STORMCHASER_OIDC_REQUIRE_PKCE` | `true` | `false`, `FALSE` or `0` lets a login go ahead without a PKCE challenge. |
| `STORMCHASER_OIDC_EXTRA_SCOPES` | unset | Scopes to ask the identity provider for on top of `openid profile email offline_access`, separated by spaces or commas. Dex needs `groups` for group-based policy. Helm: `global.oidc.extraScopes`. |
| `API_BASE_URL` | `http://localhost:3000` | Base URL the MCP server's tools call back to. Also read by the query service's shared config, which does not use it. |
| `MCP_ALLOWED_HOSTS` | empty | Comma-separated extra `Host` header values the MCP server accepts. Helm: `api.mcpAllowedHosts`. |
| `STORMCHASER_API_DOCS` | off | `true` (any case) serves Swagger UI at `/swagger-ui` and the OpenAPI document. Helm: `api.docs`. |
| `API_RATE_LIMIT_PER_SECOND` | `5` | Rate at which a client's request bucket refills. Helm: `api.rateLimitPerSecond`. |
| `API_RATE_LIMIT_BURST_SIZE` | `10` | Size of a client's request bucket. Helm: `api.rateLimitBurstSize`. |
| `STORMCHASER_TRUSTED_PROXIES` | empty | Comma-separated addresses or CIDR ranges of reverse proxies whose `X-Forwarded-For` is believed for rate limiting. Helm: `api.trustedProxies`. |
| `STORMCHASER_WEBHOOK_DEDUPE_HOURS` | `24` | How long an accepted webhook delivery is remembered so a repeat is ignored. |
| `CRON_ENGINE` | `kubernetes` | `kubernetes` creates a CronJob per cron workflow; `none` stores the schedule and leaves firing it to you. Anything else fails cron registration. Helm: `api.cron.engine`. |
| `KUBERNETES_NAMESPACE` | `default` | Namespace cron CronJobs and their Secrets are created in. Helm: the pod's own namespace. |
| `SYSTEM_URL` | `http://stormchaser-api.default.svc.cluster.local` | Base URL cron CronJobs call the trigger endpoint on. Helm: the API Service URL, set when `api.cron.engine` is `kubernetes`. |
| `STORMCHASER_CRON_CA_FILE` | unset | CA file the cron trigger trusts to reach an API that serves TLS itself. Helm: `global.tls.caCertPath` when the services listen on TLS. |
| `STORMCHASER_CRON_IMAGE` | `curlimages/curl:8.14.1@sha256:...` (pinned) | Image the cron trigger runs. |

### Engine (`stormchaser-engine`)

| Variable | Default | Description |
|---|---|---|
| `GIT_CACHE_DIR` | `/tmp/stormchaser/git-cache` | Where the engine keeps cloned repositories. Helm: `engine.gitCacheDir`. |
| `STORMCHASER_ARCHIVE_RETENTION` | unset | How long finished runs are kept after they are archived, such as `30d` (at least `1h`). The leader deletes older archived runs every hour. Unset keeps them; an invalid value is logged and keeps them. Objects in storage (SFS archives, reports, artifacts) are not deleted. |
| `VAULT_ADDR` | `http://localhost:8200` | Vault server for `secret()` lookups. |
| `VAULT_TOKEN` | unset | Vault token. Without one, `secret()` lookups are refused. |
| `STORMCHASER_VAULT_ALLOWED_PATHS` | empty | Comma-separated secret paths workflows may read with `secret()`; a trailing `*` matches a prefix. Empty refuses every lookup. |
| `STORMCHASER_AWS_ALLOWED_LAMBDA_FUNCTIONS` | empty | Comma-separated Lambda names or ARNs the engine may invoke with its own credentials; a trailing `*` matches a prefix. Helm: `global.aws.allowedLambdaFunctions`. |
| `STORMCHASER_ENV_ALLOWLIST` | empty | Comma-separated engine environment variables workflows may read with `env()`. Empty refuses all. |
| `STORMCHASER_ASSIGNED_SHARDS` | `0` | Comma-separated shard IDs this engine replica consumes. An empty or non-numeric entry stops the engine. |
| `STORMCHASER_NATS_STREAM_MAX_AGE_HOURS` | `168` | How long the NATS stream keeps a message; `0` means no limit. |
| `STORMCHASER_RUNNER_SIGNATURES` | `required` | `optional` (or `OPTIONAL`) accepts unsigned messages from runners that have no key yet. Anything else requires signatures. |
| `STORMCHASER_APPROVAL_LINK_TTL_HOURS` | `72` | How long an approval link stays valid. |
| `SYSTEM_URL` | `http://localhost:3000` | Public base URL put in approval links. Not set by the chart. |
| `SMTP_SERVER` | `localhost` | SMTP server for email steps and approval emails. The `SMTP_*` variables apply only to an engine built with the `email` feature. |
| `SMTP_PORT` | `25` | SMTP port. |
| `SMTP_USERNAME` | unset | SMTP user. |
| `SMTP_PASSWORD` | unset | SMTP password. |
| `SMTP_USE_TLS` | off | Exactly `true` turns on TLS. |
| `SMTP_USE_MTLS` | off | Exactly `true` turns on TLS and presents the engine's client certificate. |
| `STORMCHASER_WASM_TIMEOUT_SECS` | `30` | Wall-clock limit for loading and running a WASM module. |
| `STORMCHASER_WASM_MAX_MEMORY_MB` | `256` | Linear memory cap for a WASM module. |
| `STORMCHASER_WASM_MAX_CONCURRENT` | `4` | WASM modules that may run at once. |
| `STORMCHASER_MAX_ITERATIONS` | `1000` | Most step instances one `iterate` may create. |
| `STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS` | off | `true` (any case) or `1` lets the engine hand a registry's stored username and password to a runner when the registry cannot issue scoped tokens. |
| `STORMCHASER_GIT_IMAGE` | `alpine/git:v2.54.0@sha256:...` (pinned) | Image for `GitCheckout` steps. |
| `STORMCHASER_TERRAFORM_IMAGE` | `hashicorp/terraform:1.16.6@sha256:...` (pinned) | Image for Terraform steps. |
| `STORMCHASER_RUN_MAX_CONCURRENCY` | `50` | Most steps one run may have running at once. |
| `STORMCHASER_RUN_MAX_CPU` | `64` | Most CPU one run's steps may claim together. |
| `STORMCHASER_RUN_MAX_MEMORY` | `256Gi` | Most memory one run's steps may claim together. |

### Query service (`stormchaser-query`)

The query service builds its configuration with the API's code, so it reads the API
variables in the shared table above (database, TLS, NATS, OPA, OIDC, JWT secret).

| Variable | Default | Description |
|---|---|---|
| `PORT` | `3001` | Port the service listens on. Helm: fixed at `3001`. |
| `STORMCHASER_QUERY_MAX_HYDRATIONS` | `64` | Most schema hydrations in flight across the service; more are refused. |
| `STORMCHASER_QUERY_MAX_HYDRATIONS_PER_USER` | `4` | Most hydrations one user may have in flight. |

### Web (`stormchaser-web`)

| Variable | Default | Description |
|---|---|---|
| `API_URL` | `http://127.0.0.1:3000` | API URL the server side calls. Helm: the API Service URL. |
| `EXTERNAL_API_URL` | `API_URL` | API URL browsers are sent to for login. Helm: `web.externalApiUrl`. |
| `WEB_URL` | `http://127.0.0.1:3001` | Public URL of the web tier, used for the login callback. An `https://` value also turns on HSTS. Helm: `web.webUrl` (chart default `http://localhost:3003`). |
| `QUERY_URL` | `http://127.0.0.1:3001` | Query service URL for schema hydration. Helm: the query Service URL. |
| `GRAFANA_URL` | unset | Grafana URL embedded in pages and allowed in the content security policy. Must be `http` or `https`. Helm: `global.grafana.url`. |
| `LEPTOS_WATCH` | unset | Development only: when set, the content security policy allows the live-reload connection. |

The web tier also reads `TLS_CA_CERT_PATH` (trusted for its calls to the API and the
query service, and a file it cannot read stops it), `TLS_CERT_PATH`, `TLS_KEY_PATH` and
`STORMCHASER_LISTEN_TLS` from the shared table.

### Docker runner (`stormchaser-runner-docker`)

| Variable | Default | Description |
|---|---|---|
| `STORMCHASER_STEP_NETWORK` | unset | Docker network step containers join. Unset, steps share the runner's network and a warning is logged. Helm: runner-docker `config.stepNetwork`. |
| `STORMCHASER_SFS_HOST_PATH` | unset | Host directory that step storage is bind-mounted from. Set, storage is not parked to object storage. |
| `STORMCHASER_STEP_PIDS_LIMIT` | `4096` | Most processes and threads a step's container may have. Helm: runner-docker `config.stepLimits.pidsLimit`. |

### Kubernetes runner (`stormchaser-runner-k8s`)

| Variable | Default | Description |
|---|---|---|
| `STORMCHASER_STEP_NAMESPACE` | none, **required** | Namespace step Jobs run in. The runner will not start if it is unset, a system namespace, or its own namespace. Helm: runner-k8s `rbac.targetNamespace` (default `<release namespace>-steps`). |
| `KUBERNETES_NAMESPACE` | unset | The runner's own namespace, checked against the step namespace. Unset, it is read from the service account mount. Helm: the pod's namespace. |
| `STORMCHASER_STEP_MAX_PARALLELISM` | `10` | Most pods one step's Job may run at once or ask to complete. Helm: runner-k8s `config.stepLimits.maxParallelism`. |
| `STORMCHASER_STEP_IMAGE_PULL_POLICY` | `Always` | `Always`, `IfNotPresent` or `Never` for steps' images. Anything else logs a warning and uses `Always`. Helm: runner-k8s `config.stepImagePullPolicy`. |
| `STORMCHASER_ALLOWED_SERVICE_ACCOUNTS` | empty | Comma-separated ServiceAccounts a step may request; a trailing `*` matches a prefix. Helm: runner-k8s `config.allowedServiceAccounts`. |
| `STORMCHASER_ALLOWED_SECRET_MOUNTS` | empty | Comma-separated Secrets a step may mount. Helm: runner-k8s `config.allowedSecretMounts`. |
| `STORMCHASER_ALLOWED_CONFIG_MAP_MOUNTS` | empty | Comma-separated ConfigMaps a step may mount. Helm: runner-k8s `config.allowedConfigMapMounts`. |
| `STORMCHASER_SFS_PVC_NAME` | unset | PersistentVolumeClaim step storage lives on. Set, storage is not parked to object storage. |

The runner also sets variables on step pods for its init containers and the agent
(`STORMCHASER_FETCH_URL`, `STORMCHASER_PARKING_URLS`, `STORMCHASER_ARTIFACT_URLS`,
`STORMCHASER_PROVISION_DEST`, `STORMCHASER_PROVISION_MODE`). They are internal, not
settings.

### Agent (`stormchaser-agent`)

The runners start the agent and set these. They are the `run` subcommand's
`--test-reports`, `--report-urls` and `--registry-config` options.

| Variable | Default | Description |
|---|---|---|
| `STORMCHASER_TEST_REPORTS` | unset | JSON list of test reports to collect (name, path, format). |
| `STORMCHASER_REPORT_URLS` | unset | JSON map of report name to upload URL and destination. |
| `STORMCHASER_REGISTRY_CONFIG` | unset | Docker client configuration file with credentials for OCI artifact pushes. |

### CLI and TUI (`stormchaser`, `stormchaser-tui`)

| Variable | Default | Description |
|---|---|---|
| `STORMCHASER_URL` | CLI: the profile's URL, then `http://localhost:3000`; TUI: `http://localhost:3000` | API base URL (`--url`). |
| `STORMCHASER_TOKEN` | unset | API token (`--token`). Prefer this to the flag, which other users on the host can see. |
| `STORMCHASER_CA_CERT` | unset | PEM file of CA certificates to trust as well as the system's (`--ca-cert`). |
| `STORMCHASER_QUERY_URL` | `http://localhost:3001` | TUI only: query service URL (`--query-url`). |
| `STORMCHASER_TUI_DEV_LOGIN` | off | TUI only, development: exactly `true` offers logins from `deploy/dex/credentials.generated` under the working directory. |
| `HOST_DEX` | unset | TUI only, development: host name that replaces `localhost` or `127.0.0.1` in the Dex login URL. |

### Set by deployment files but not read by any code

- `BINARY`, set on every pod by the charts. The image runs the binary named by
  `BINARY_NAME`, fixed when the image is built.
- `GRAFANA_URL` on the API, engine and query pods (chart). Only the web tier reads it.
- `OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_SERVICE_NAME` on the query service (chart and
  compose). The query service does not set up OpenTelemetry.
- `KUBERNETES_NAMESPACE` on the query service (chart). Only the API's cron code and the
  Kubernetes runner read it.
- `API_RATE_LIMIT_PER_SECOND` and `API_RATE_LIMIT_BURST_SIZE` on the query service
  (compose). Rate limiting is in the API's router, which the query service does not use.
- `RUST_LOG` on the web tier (chart and compose). The web tier sets up no logger.
- `S3_INTERNAL_URL` on the engine (compose).
