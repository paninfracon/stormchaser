# Stormchaser Kubernetes Environments

Stormchaser supports three primary deployment models: **Docker Only**,
**Hybrid Local (Docker + MicroK8s)**, and **Full Kubernetes (Helm)**.

---

## 1. Quick Start: Full Stack with MicroK8s

This is the fastest way to get a production-like environment running on your
local machine, complete with automated log management and monitoring
(Prometheus/Grafana).

### Quick Start Prerequisites

- **MicroK8s**: Install via `snap install microk8s --classic`.
- **Enable Addons**:

  ```bash
  microk8s enable dns rbac storage ingress
  ```

- **Helm**: Ensure `helm` is installed locally.

### Step 1: Run the Deployment Script

The provided `setup.sh` script automates the process of building the
Stormchaser Docker images, generating required TLS certificates, configuring
MicroK8s CRDs, and deploying the full stack via the umbrella Helm chart.

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
- **MicroK8s**: Ensure the `dns` and `rbac` addons are enabled
  (`microk8s enable dns rbac`).

### Setup

The local setup script handles certificate generation, bridging the networks,
and runner deployment.

```bash
# Start in Hybrid mode (Default)
./scripts/setup.sh --mode k8s

# Full reset and restart
./scripts/setup.sh --cleanup --mode k8s
```

### Hybrid Architecture

- **Docker Compose**: Runs core infrastructure (Postgres, NATS, Dex, Loki,
  MinIO) and Orchestration services (API, Engine).
- **MicroK8s**: Runs the `stormchaser-runner-k8s` and the actual workflow steps
  (as Kubernetes Jobs).

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
  cluster).

### Step 1: Provision the Namespace and Secrets

First, create a namespace for Stormchaser and populate the necessary secrets
(TLS certificates, Database credentials, etc.).

```bash
kubectl create namespace stormchaser

# Example: Create the database connection secret
kubectl create secret generic stormchaser-db-secret \
  --namespace stormchaser \
  --from-literal=DATABASE_URL="postgres://user:password@host:5432/db"

# Example: Create the TLS secret for mTLS
kubectl create secret tls stormchaser-tls-certs \
  --namespace stormchaser \
  --cert=path/to/tls.crt \
  --key=path/to/tls.key
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
  --set global.nats.url="nats://my-nats-cluster:4222" \
  --set global.oidc.issuer="https://my-identity-provider/dex"
```

### Step 3: Install the K8s Runner

The runner chart deploys the execution agent responsible for managing
Kubernetes Jobs for workflow steps. It needs to communicate with the NATS
cluster.

Review the `deploy/charts/stormchaser-runner-k8s/values.yaml` file to configure
NATS and runner behavior.

```bash
helm install stormchaser-runner ./deploy/charts/stormchaser-runner-k8s \
  --namespace stormchaser \
  --set config.natsUrl="nats://my-nats-cluster:4222" \
  --set rbac.create=true
```

*Note: The runner chart creates a ServiceAccount and RoleBinding to allow it to
spawn and monitor Jobs and Pods in the target namespace.*

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
# 1. Stop and remove Docker Compose resources
docker compose down -v --remove-orphans

# 2. Prune Docker system (removes stopped containers, dangling images, build cache)
docker system prune -f

# 3. Clean Cargo build artifacts
cargo clean

# 4. Reset MicroK8s (requires elevated privileges)
sudo microk8s reset

# 5. Remove temporary local directories
rm -rf .tmp scripts/.tmp
```
