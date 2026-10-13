# NATS: Quickstart and Production

NATS JetStream carries every task Stormchaser hands to a runner and every
result a runner reports back. Anyone who can publish to it can schedule
containers and report on other users' steps, so how NATS is secured decides how
far a compromised step or runner can reach.

Stormchaser bundles NATS in two places so that you can evaluate it without
provisioning anything: `docker-compose.yml` and the umbrella Helm chart
(`deploy/charts/stormchaser`). **The bundled server is a quickstart, not a
production broker.** For production, run NATS yourself (or use a managed
service) and configure it as described [below](#production-requirements).

## What the quickstart gives you

| | Quickstart (compose and umbrella chart) | Production requirement |
| --- | --- | --- |
| Authentication | Two users with generated passwords | Yes, per role |
| Runner permissions | Restricted to the subjects below | Yes |
| TLS | None; traffic is plaintext | TLS on every link, verified |
| Nodes | 1 | 3 or more, clustered |
| Stream replicas | 1 | 3 |
| Network isolation | Steps on a separate Docker network or behind a NetworkPolicy; **off** when started by `scripts/setup.sh` | Step containers cannot reach NATS |
| Monitoring port (8222) | Open; published on the host by compose | Restricted |
| Storage | 10Gi PVC (Helm); container filesystem (compose) | Sized, persistent, encrypted |

The quickstart requires a password because evaluation installs have a habit of
becoming permanent. That makes it safe against a stray client on the same
network. It does not make it production-ready: passwords and every message
travel in plaintext, and a single node loses in-flight work when it restarts.

### Quickstart credentials

| User | Used by | Where the password lives |
| --- | --- | --- |
| `stormchaser` | API, engine, query service | compose: `STORMCHASER_NATS_PASSWORD` in `.env`; Helm: Secret `stormchaser-nats-control` |
| `stormchaser-runner` | Docker and Kubernetes runners | compose: `STORMCHASER_NATS_RUNNER_PASSWORD` in `.env`; Helm: Secret `stormchaser-nats-runner` |

`scripts/setup.sh` generates the compose passwords. The umbrella chart
generates the Helm Secrets on first install and keeps them across upgrades
(`natsAuth.generate`). The server configuration is in
`deploy/nats/nats-server.conf` for compose and under `nats.config.merge` in the
umbrella chart's `values.yaml`; keep the two in step.

To talk to the quickstart server yourself, pass the control-plane credentials:

```bash
nats --server nats://localhost:4222 \
  --user stormchaser --password "$STORMCHASER_NATS_PASSWORD" stream ls
```

## How components authenticate

Every component reads its NATS credentials from the environment, in this order
of preference:

| Variable | Meaning |
| --- | --- |
| `NATS_CREDS_FILE` | Path to a NATS `.creds` file (JWT and NKey). Preferred for production. |
| `NATS_USER` / `NATS_PASSWORD` | Username and password. |
| `NATS_TOKEN` | Shared token. NATS documents this as suitable for quick internal setups only. |
| `NATS_CA_FILE` | Root certificate used to verify the NATS server. |
| `NATS_TLS_REQUIRED` | `true` refuses a server that does not offer TLS. |

A component that finds no credentials logs a warning and connects anonymously.
Credentials embedded in `NATS_URL` (`nats://user:pass@host`) are **not** used.

In Helm, `global.nats.authSecret` (orchestration chart) and
`config.natsAuthSecret` (runner charts) name a Secret with `user` and
`password` keys. To use a `.creds` file or TLS settings, mount the file and set
the variables above on the deployment.

The API and engine also present their mTLS client certificate
(`TLS_CERT_PATH` / `TLS_KEY_PATH`) to NATS. It is only checked if the server
sets `verify: true`. The runners do not present a client certificate; they
authenticate with the credentials above.

## What Stormchaser needs from NATS

- JetStream enabled.
- One stream, `stormchaser`, capturing `stormchaser.>`. The engine creates it
  if it does not exist; you can create it yourself (with the replica count you
  want) before first start.
- Durable pull consumers, created by the components that use them:
  `orchestration-engine-global`, `orchestration-engine-shard-<n>` (one per
  engine shard), `orchestration-engine-legacy-run`,
  `orchestration-engine-legacy-runner`, `orchestration-engine-legacy-step`,
  `docker-runner`, `k8s-runner`, and one ephemeral consumer per running runner
  named `runner-<type>-<runner id>`.
- A key-value bucket for API rate limiting, created by the API.

## Production requirements

### 1. Separate users with scoped permissions

Give the control plane and the runners different identities. NATS treats a user
with no `permissions` block as unrestricted, and a grant of `>` includes the
JetStream control plane under `$JS.API.>`, so the runner user must have an
explicit allow list.

**Control plane** (API, engine, query service): publish and subscribe on
`stormchaser.>`, `$JS.API.>`, `$JS.ACK.>`, `$KV.>` and `_INBOX.>`.

**Runners**: exactly the list below. It is the one the quickstart ships, and it
has been checked against a NATS 2.10 server: a runner starts, registers,
creates its consumers and pulls work with no permission errors, and is refused
when it tries to subscribe to `stormchaser.>`, create a consumer on any other
subject, pull from an engine consumer, publish a `run.*` or `step.scheduled.*`
event, or delete or read the stream directly.

```text
publish allow:
  stormchaser.v1.runner.register
  stormchaser.v1.global.runner.heartbeat
  stormchaser.v1.global.runner.offline
  stormchaser.v1.*.step.initializing
  stormchaser.v1.*.step.running
  stormchaser.v1.*.step.completed
  stormchaser.v1.*.step.failed
  stormchaser.v1.step.unpacking_sfs
  stormchaser.v1.step.packing_sfs
  stormchaser.v1.step.query
  $JS.API.STREAM.INFO.stormchaser
  $JS.API.STREAM.CREATE.stormchaser
  $JS.API.CONSUMER.INFO.stormchaser.*
  $JS.API.CONSUMER.CREATE.stormchaser.*.stormchaser.v1.*.step.scheduled.>
  $JS.API.CONSUMER.CREATE.stormchaser.*.stormchaser.v1.runner.>
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.*
  $JS.ACK.stormchaser.>

publish deny (the engine's own consumers; one shard entry per engine shard):
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-global
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-legacy-run
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-legacy-runner
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-legacy-step
  $JS.API.CONSUMER.MSG.NEXT.stormchaser.orchestration-engine-shard-0
  $JS.ACK.stormchaser.orchestration-engine-global.>
  $JS.ACK.stormchaser.orchestration-engine-legacy-run.>
  $JS.ACK.stormchaser.orchestration-engine-legacy-runner.>
  $JS.ACK.stormchaser.orchestration-engine-legacy-step.>
  $JS.ACK.stormchaser.orchestration-engine-shard-0.>

subscribe allow:
  _INBOX.>
```

If you create the stream yourself, drop `$JS.API.STREAM.CREATE.stormchaser`
from the runner list. If you run more than one engine shard
(`STORMCHASER_TOTAL_SHARDS`), add the two deny entries for each shard number.

Prefer NKeys or operator-mode `.creds` files to passwords: no secret crosses
the wire. If you use passwords, store bcrypt hashes in the server
configuration and require TLS, because the client still sends the password
itself.

**What these permissions do not stop.** A runner credential can report
`completed` or `failed` for any step, including approval steps, because the
engine cannot yet tell which runner a report came from. It can also read tasks
scheduled for other runners of any type. Treat every runner host as trusted to
that extent, and see the remediation plan for the planned fix.

### 2. TLS on every link

- Enable TLS for client connections and, separately, for cluster routes.
  Enabling it for clients leaves routes in plaintext until the `cluster` block
  is configured too.
- Set `verify: true` on the client block if you want NATS to require client
  certificates. Without it, any client that trusts your CA connects without
  presenting one.
- Set `NATS_TLS_REQUIRED=true` and, for a private CA, `NATS_CA_FILE` on every
  Stormchaser component, and use a `tls://` URL.

### 3. Keep workflow steps away from NATS

Workflow authors choose the images that steps run. A step that can open a
connection to NATS can attack it directly, so the NATS client port must not be
reachable from step containers:

- **Kubernetes**: steps run in their own namespace, and the runner chart
  applies a NetworkPolicy there (`networkPolicy.enabled`, on by default). It
  denies all ingress and allows egress only to cluster DNS and to addresses
  outside the private, carrier-grade NAT and link-local ranges, so NATS,
  PostgreSQL, the Kubernetes API and the cloud metadata address
  (`169.254.169.254`) are out of reach. Check three things in your cluster:
  that the CNI enforces NetworkPolicy; that `networkPolicy.blockedCidrs`
  covers your pod and service ranges; and that cluster DNS is not served from
  a link-local address (NodeLocal DNSCache), which the default ranges block.
  If steps must reach an in-cluster object store, allow just that with
  `networkPolicy.allowReleaseNamespace`, as the umbrella chart does for MinIO.
- **Docker runner**: set `STORMCHASER_STEP_NETWORK` on the runner to a Docker
  network that has no route to NATS or PostgreSQL; step containers join it
  instead of the runner's own network. A network on the same host can still
  reach any port the control plane publishes on the host, so do not publish
  NATS or PostgreSQL beyond loopback on runner hosts. The bundled compose
  file does both: steps join a `steps` network that only object storage is
  attached to, and the PostgreSQL, NATS and OPA ports are published on
  `127.0.0.1` (`STORMCHASER_CONTROL_PLANE_BIND`).

`scripts/setup.sh` turns this isolation **off** for local development, because
the dogfood workflows run the test suite and deploy charts from inside steps.
Set `STORMCHASER_ISOLATE_STEPS=true` before running it to keep isolation on.

Also restrict the monitoring port (8222): it serves connection, user and
subject details in plaintext. On Kubernetes keep it bound for probes and limit
it with a NetworkPolicy; elsewhere bind it to localhost.

### 4. Availability and storage

- Run at least three clustered nodes and give the `stormchaser` stream three
  replicas. Stormchaser does not set a replica count, so a stream it creates
  gets the server default of one; create the stream yourself, or update it,
  to raise that.
- Use persistent, adequately sized file storage. Set a container memory limit
  above the JetStream memory-store size, and `GOMEMLIMIT` a little below the
  limit.
- For encryption at rest, NATS recommends filesystem-level encryption of the
  store directory over its built-in option.

### 5. Deploying against your own NATS with Helm

```bash
kubectl create secret generic nats-control -n stormchaser \
  --from-literal=user=stormchaser --from-literal=password='<control password>'
kubectl create secret generic nats-runner -n stormchaser \
  --from-literal=user=stormchaser-runner --from-literal=password='<runner password>'

helm install stormchaser-orchestration ./deploy/charts/stormchaser-orchestration \
  --namespace stormchaser \
  --set global.nats.url="tls://nats.example.internal:4222" \
  --set global.nats.authSecret=nats-control

helm install stormchaser-runner ./deploy/charts/stormchaser-runner-k8s \
  --namespace stormchaser \
  --set config.natsUrl="tls://nats.example.internal:4222" \
  --set config.natsAuthSecret=nats-runner
```

With the umbrella chart, set `nats.enabled=false` and
`natsAuth.generate=false`, and point `stormchaser-orchestration.global.nats`
and `stormchaser-runner-k8s.config` at your server and Secrets.

### Checklist

- [ ] NATS rejects anonymous connections.
- [ ] Control plane and runners use different credentials.
- [ ] The runner user has the allow and deny lists above, with a deny entry
      for every engine shard.
- [ ] TLS is on for clients and routes; components set `NATS_TLS_REQUIRED`.
- [ ] Step containers and pods cannot reach port 4222.
- [ ] Port 8222 is not reachable outside the NATS hosts or namespace.
- [ ] Three or more nodes; the `stormchaser` stream has three replicas.
- [ ] The store directory is on persistent, encrypted storage.
